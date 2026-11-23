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

    let delivery = Delivery {
        severity: alarm.severity.clone(),
        source_key: alarm.source_key.clone(),
        source_name: source_name.clone(),
        value: alarm.value,
        threshold: alarm.threshold,
        unit: source_info.map(|s| s.unit.clone()).unwrap_or_default(),
        message: alarm.message.clone(),
        triggered_at: triggered_at_str.clone(),
        device_id: device_id.clone(),
    };
    for err in deliver_to_settings(&settings, &delivery, email_notifier.as_ref()).await {
        tracing::error!("Alarm notification for {}: {}", alarm.source_key, err);
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

/// One notification, whatever caused it (an alarm, or a flow's `notify` node).
#[derive(Debug, Clone)]
pub struct Delivery {
    pub severity: String,
    pub source_key: String,
    pub source_name: String,
    pub value: f64,
    pub threshold: f64,
    pub unit: String,
    pub message: String,
    /// RFC 3339 time shown to the recipient.
    pub triggered_at: String,
    pub device_id: String,
}

/// Send `delivery` through every enabled setting in `settings` (email,
/// webhook, and the edition's chat channels). Returns one message per failed
/// recipient; an empty list means everything was sent.
pub async fn deliver_to_settings(
    settings: &[crate::models::NotificationSetting],
    delivery: &Delivery,
    email_notifier: Option<&EmailNotifier>,
) -> Vec<String> {
    let mut errors = Vec::new();
    for setting in settings {
        if !setting.enabled {
            continue;
        }
        let targets =
            || -> Vec<String> { serde_json::from_str(&setting.config).unwrap_or_default() };
        match setting.channel_type.as_str() {
            "email" => {
                let Some(notifier) = email_notifier else {
                    errors.push("email requested but SMTP is not configured".to_string());
                    continue;
                };
                // config is a JSON array of recipient email addresses
                for to in targets() {
                    if let Err(e) = send_email(notifier, &to, delivery).await {
                        errors.push(format!("email to {to}: {e}"));
                    }
                }
            }
            "webhook" => {
                // config is a JSON array of webhook URLs
                for url in targets() {
                    if let Err(e) = send_webhook(&url, delivery).await {
                        errors.push(format!("webhook {url}: {e}"));
                    }
                }
            }
            channel @ ("telegram" | "whatsapp") => {
                let Some(sender) = crate::edition::chat_sender() else {
                    errors.push(format!(
                        "{channel} requested but chat delivery is not available in this edition"
                    ));
                    continue;
                };
                let chat_alarm = crate::edition::ChatAlarm {
                    severity: delivery.severity.clone(),
                    source_key: delivery.source_key.clone(),
                    source_name: delivery.source_name.clone(),
                    value: delivery.value,
                    threshold: delivery.threshold,
                    unit: delivery.unit.clone(),
                    message: delivery.message.clone(),
                    triggered_at: delivery.triggered_at.clone(),
                };
                match sender.send(channel, &setting.config, &chat_alarm).await {
                    Ok(deliveries) => {
                        for d in deliveries {
                            if let Some(err) = d.error {
                                errors.push(format!("{channel} to {}: {err}", d.target));
                            }
                        }
                    }
                    Err(e) => errors.push(format!("{channel}: {e}")),
                }
            }
            other => errors.push(format!("unknown notification channel type: {other}")),
        }
    }
    errors
}

/// Send `delivery` as an email to one address.
pub async fn send_email(
    notifier: &EmailNotifier,
    to: &str,
    delivery: &Delivery,
) -> anyhow::Result<()> {
    let data = AlarmEmailData {
        source_key: delivery.source_key.clone(),
        source_name: delivery.source_name.clone(),
        severity: delivery.severity.clone(),
        value: delivery.value,
        threshold: delivery.threshold,
        message: delivery.message.clone(),
        unit: delivery.unit.clone(),
        triggered_at: delivery.triggered_at.clone(),
    };
    notifier.send_alarm_email(to, &data).await
}

/// POST `delivery` to one webhook URL (alarm webhook payload format).
pub async fn send_webhook(url: &str, delivery: &Delivery) -> anyhow::Result<()> {
    let payload = WebhookPayload {
        severity: delivery.severity.clone(),
        source_key: delivery.source_key.clone(),
        source_name: delivery.source_name.clone(),
        value: delivery.value,
        threshold: delivery.threshold,
        message: delivery.message.clone(),
        triggered_at: delivery.triggered_at.clone(),
        device_info: delivery.device_id.clone(),
    };
    send_webhook_alarm(url, &payload).await
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
    fire_outbound_webhooks(storage, &webhooks, payload);
}

/// One HTTP client for every outbound webhook, so deliveries share its
/// connection pool instead of each building its own.
static WEBHOOK_CLIENT: std::sync::LazyLock<reqwest::Client> =
    std::sync::LazyLock::new(reqwest::Client::new);

/// Outbound webhook deliveries (with their retries) running at once. Past
/// this, new deliveries are dropped and counted rather than queued without
/// bound behind a slow or unreachable receiver.
const MAX_WEBHOOK_DELIVERIES: usize = 32;

static WEBHOOK_SLOTS: std::sync::LazyLock<std::sync::Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| {
        std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_WEBHOOK_DELIVERIES))
    });

static WEBHOOKS_DROPPED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Remembers the last value and quality sent per tag, so `tag_change`
/// webhooks fire when a tag changes rather than on every persistence cycle.
#[derive(Default)]
pub struct TagChangeTracker {
    last: std::collections::HashMap<(String, String), (u64, crate::quality::Quality)>,
}

impl TagChangeTracker {
    /// True when the tag's value or quality differs from the last call (or
    /// this is the first one), and records it.
    pub fn changed(
        &mut self,
        device_id: &str,
        metric: &str,
        value: f64,
        quality: crate::quality::Quality,
    ) -> bool {
        // Compared by bits so a NaN that stays NaN is not a change.
        let now = (value.to_bits(), quality);
        match self
            .last
            .get_mut(&(device_id.to_string(), metric.to_string()))
        {
            Some(prev) if *prev == now => false,
            Some(prev) => {
                *prev = now;
                true
            }
            None => {
                self.last
                    .insert((device_id.to_string(), metric.to_string()), now);
                true
            }
        }
    }

    /// Forget tags of devices that are no longer present.
    pub fn retain_devices(&mut self, present: &std::collections::HashSet<&str>) {
        self.last.retain(|(d, _), _| present.contains(d.as_str()));
    }
}

/// Fire `payload` at the already-loaded `webhooks` that subscribe to its
/// event (see `dispatch_outbound_webhooks`).
pub fn fire_outbound_webhooks(
    storage: &Storage,
    webhooks: &[serde_json::Value],
    payload: &serde_json::Value,
) {
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
        let Ok(slot) = WEBHOOK_SLOTS.clone().try_acquire_owned() else {
            let n = WEBHOOKS_DROPPED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            if n == 1 || n.is_multiple_of(1000) {
                tracing::warn!(
                    "Outbound webhook {wh_id} skipped: {MAX_WEBHOOK_DELIVERIES} deliveries \
                     already in progress ({n} skipped so far)"
                );
            }
            continue;
        };
        let storage_clone = storage.clone();
        let payload_clone = payload.clone();
        tokio::spawn(async move {
            let _slot = slot;
            let client = &*WEBHOOK_CLIENT;
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

#[cfg(test)]
mod tag_change_tests {
    use super::TagChangeTracker;
    use crate::quality::Quality;

    #[test]
    fn fires_on_change_only() {
        let mut t = TagChangeTracker::default();
        assert!(t.changed("plc", "temp", 20.0, Quality::Good));
        assert!(!t.changed("plc", "temp", 20.0, Quality::Good));
        assert!(t.changed("plc", "temp", 20.5, Quality::Good));
        assert!(t.changed("plc", "temp", 20.5, Quality::Uncertain));
        assert!(t.changed("plc", "temp", f64::NAN, Quality::Bad));
        assert!(!t.changed("plc", "temp", f64::NAN, Quality::Bad));
        assert!(t.changed("plc2", "temp", 20.5, Quality::Uncertain));

        let present = std::collections::HashSet::from(["plc2"]);
        t.retain_devices(&present);
        assert!(t.changed("plc", "temp", f64::NAN, Quality::Bad));
        assert!(!t.changed("plc2", "temp", 20.5, Quality::Uncertain));
    }
}
