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
        // Shown with the tag's decimals, not the raw float (2.602250099182129).
        value: source_info.map_or(alarm.value, |s| {
            crate::alarm_eval::round_to_decimals(alarm.value, s.decimals)
        }),
        threshold: source_info.map_or(alarm.threshold, |s| {
            crate::alarm_eval::round_to_decimals(alarm.threshold, s.decimals)
        }),
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
                        let url = crate::security::redact::mask_url(&url);
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
    dispatch_outbound_webhooks_with(&WebhookSender::policy(), storage, event, payload).await;
}

async fn dispatch_outbound_webhooks_with(
    sender: &WebhookSender,
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
    // Alarm (and other rare) events are never dropped: each delivery waits
    // for one of the alarm slots, which tag_change traffic does not use.
    for target in webhook_targets(&webhooks, payload) {
        let sender = sender.clone();
        let storage = storage.clone();
        let payload = payload.clone();
        tokio::spawn(async move {
            let Ok(_slot) = ALARM_SLOTS.clone().acquire_owned().await else {
                return;
            };
            deliver(&sender, &storage, &target, &payload).await;
        });
    }
}

/// One outbound-policy HTTP client for every webhook delivery, so they share
/// its connection pool instead of each building its own.
static WEBHOOK_CLIENT: std::sync::LazyLock<reqwest::Client> =
    std::sync::LazyLock::new(|| crate::security::outbound::client(None));

/// Alarm webhook deliveries (with their retries) running at once. More
/// wait their turn; none is dropped.
const MAX_ALARM_DELIVERIES: usize = 16;

static ALARM_SLOTS: std::sync::LazyLock<std::sync::Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| {
        std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_ALARM_DELIVERIES))
    });

/// One webhook a payload goes to, read from its stored row.
#[derive(Debug, Clone)]
pub struct WebhookTarget {
    id: i64,
    url: String,
    headers: std::collections::HashMap<String, String>,
    retry_count: u32,
    timeout_ms: u64,
}

/// The webhooks of `webhooks` whose device filter lets `payload` through.
pub fn webhook_targets(
    webhooks: &[serde_json::Value],
    payload: &serde_json::Value,
) -> Vec<WebhookTarget> {
    let payload_device = payload
        .get("device_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    webhooks
        .iter()
        .filter_map(|wh| {
            let url = wh.get("url").and_then(|v| v.as_str())?.to_string();
            let device_filter = wh
                .get("device_filter")
                .and_then(|v| v.as_str())
                .unwrap_or("\"all\"");
            // Device filter: "all" or JSON array of device IDs
            if device_filter != "\"all\"" && device_filter != "all" {
                let filter_str = device_filter.trim_matches('"');
                let allowed: Vec<String> = if filter_str.starts_with('[') {
                    serde_json::from_str::<Vec<String>>(filter_str).unwrap_or_default()
                } else {
                    filter_str.split(',').map(|s| s.to_string()).collect()
                };
                if !allowed.is_empty() && !allowed.iter().any(|d| d == payload_device) {
                    return None;
                }
            }
            Some(WebhookTarget {
                id: wh.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
                url,
                headers: wh
                    .get("headers")
                    .and_then(|v| v.as_str())
                    .and_then(|h| serde_json::from_str(h).ok())
                    .unwrap_or_default(),
                retry_count: wh.get("retry_count").and_then(|v| v.as_u64()).unwrap_or(3) as u32,
                timeout_ms: wh
                    .get("timeout_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(10000),
            })
        })
        .collect()
}

/// How webhook deliveries reach the network: the outbound policy client
/// (no redirects, no proxy, DNS-checked addresses, this host refused) and
/// the URL check before each delivery. Tests swap in a plain client to
/// post to a local endpoint without loosening the global policy.
#[derive(Clone)]
pub struct WebhookSender {
    client: reqwest::Client,
    check_urls: bool,
}

impl WebhookSender {
    /// The policy client every real delivery uses.
    pub fn policy() -> Self {
        Self {
            client: WEBHOOK_CLIENT.clone(),
            check_urls: true,
        }
    }

    #[cfg(test)]
    fn unchecked_for_tests() -> Self {
        Self {
            client: reqwest::Client::new(),
            check_urls: false,
        }
    }
}

/// POST `payload` to one webhook with its retries, then record the outcome.
async fn deliver(
    sender: &WebhookSender,
    storage: &Storage,
    target: &WebhookTarget,
    payload: &serde_json::Value,
) {
    let now = crate::timeutil::now_secs();
    if sender.check_urls
        && let Err(e) = crate::security::outbound::check_url(&target.url)
    {
        tracing::warn!("Outbound webhook {} not sent: {e}", target.id);
        let _ = storage
            .update_outbound_webhook_status(target.id, Some(now), Some(format!("refused: {e}")))
            .await;
        return;
    }
    let mut success = false;
    for attempt in 0..=target.retry_count {
        let mut request = sender
            .client
            .post(&target.url)
            .json(payload)
            .timeout(std::time::Duration::from_millis(target.timeout_ms));
        for (k, v) in &target.headers {
            request = request.header(k, v);
        }
        let failure = match request.send().await {
            Ok(resp) if resp.status().is_success() => {
                tracing::info!(
                    "Outbound webhook {} fired to {} (attempt {})",
                    target.id,
                    crate::security::redact::mask_url(&target.url),
                    attempt + 1
                );
                success = true;
                break;
            }
            Ok(resp) => format!("returned {}", resp.status()),
            Err(e) => format!("error: {}", e.without_url()),
        };
        if attempt < target.retry_count {
            let delay = 500 * 2u64.pow(attempt.min(10));
            tracing::warn!(
                "Outbound webhook {} {failure}, retrying in {delay}ms",
                target.id
            );
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        }
    }
    let status_str = if success { "success" } else { "error" };
    if let Err(e) = storage
        .update_outbound_webhook_status(target.id, Some(now), Some(status_str.to_string()))
        .await
    {
        tracing::warn!("Failed to update outbound webhook status: {}", e);
    }
}

/// One tag change waiting to be delivered to its webhooks.
struct TagChangeJob {
    payload: serde_json::Value,
    targets: Vec<WebhookTarget>,
}

/// Bounded queue of tag_change deliveries, drained by a fixed set of
/// workers. A full queue refuses a change instead of growing; the caller
/// then leaves it unrecorded so the next cycle offers it again.
#[derive(Clone)]
pub struct TagChangeQueue {
    tx: tokio::sync::mpsc::Sender<TagChangeJob>,
    /// Held so the queue stays open while the queue handle lives.
    _rx: std::sync::Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<TagChangeJob>>>,
}

/// Tag changes waiting for delivery.
pub const TAG_CHANGE_QUEUE_CAPACITY: usize = 10_000;
/// Workers delivering tag changes; each sends one change at a time.
pub const TAG_CHANGE_WORKERS: usize = 8;

impl TagChangeQueue {
    pub fn start(storage: Storage, capacity: usize, workers: usize) -> Self {
        Self::start_with(WebhookSender::policy(), storage, capacity, workers)
    }

    fn start_with(
        sender: WebhookSender,
        storage: Storage,
        capacity: usize,
        workers: usize,
    ) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel::<TagChangeJob>(capacity.max(1));
        let rx = std::sync::Arc::new(tokio::sync::Mutex::new(rx));
        for _ in 0..workers {
            let rx = rx.clone();
            let storage = storage.clone();
            let sender = sender.clone();
            tokio::spawn(async move {
                loop {
                    let job = rx.lock().await.recv().await;
                    let Some(job) = job else { break };
                    for target in &job.targets {
                        deliver(&sender, &storage, target, &job.payload).await;
                    }
                }
            });
        }
        Self { tx, _rx: rx }
    }

    /// Queue `payload` for the webhooks that take it. False when the queue
    /// is full (nothing was queued).
    pub fn enqueue(&self, webhooks: &[serde_json::Value], payload: serde_json::Value) -> bool {
        let targets = webhook_targets(webhooks, &payload);
        if targets.is_empty() {
            return true;
        }
        self.tx.try_send(TagChangeJob { payload, targets }).is_ok()
    }
}

/// Remembers the last value and quality sent per tag, so `tag_change`
/// webhooks fire when a tag changes rather than on every persistence cycle.
#[derive(Default)]
pub struct TagChangeTracker {
    last: std::collections::HashMap<(String, String), (u64, crate::quality::Quality)>,
}

impl TagChangeTracker {
    /// True when the tag's value or quality differs from the last one
    /// recorded (or none is).
    pub fn changed(
        &self,
        device_id: &str,
        metric: &str,
        value: f64,
        quality: crate::quality::Quality,
    ) -> bool {
        // Compared by bits so a NaN that stays NaN is not a change.
        self.last
            .get(&(device_id.to_string(), metric.to_string()))
            .is_none_or(|prev| *prev != (value.to_bits(), quality))
    }

    /// Record what was queued for a tag.
    pub fn record(
        &mut self,
        device_id: &str,
        metric: &str,
        value: f64,
        quality: crate::quality::Quality,
    ) {
        self.last.insert(
            (device_id.to_string(), metric.to_string()),
            (value.to_bits(), quality),
        );
    }

    /// Forget tags of devices that are no longer present.
    pub fn retain_devices(&mut self, present: &std::collections::HashSet<&str>) {
        self.last.retain(|(d, _), _| present.contains(d.as_str()));
    }
}

#[cfg(test)]
mod tag_change_tests {
    use super::*;
    use crate::quality::Quality;

    #[test]
    fn changed_compares_with_what_was_recorded() {
        let mut t = TagChangeTracker::default();
        assert!(t.changed("plc", "temp", 20.0, Quality::Good));
        // Not recorded (not queued): still a change next time.
        assert!(t.changed("plc", "temp", 20.0, Quality::Good));
        t.record("plc", "temp", 20.0, Quality::Good);
        assert!(!t.changed("plc", "temp", 20.0, Quality::Good));
        assert!(t.changed("plc", "temp", 20.5, Quality::Good));
        assert!(t.changed("plc", "temp", 20.0, Quality::Uncertain));
        t.record("plc", "temp", f64::NAN, Quality::Bad);
        assert!(!t.changed("plc", "temp", f64::NAN, Quality::Bad));

        t.record("plc2", "temp", 1.0, Quality::Good);
        t.retain_devices(&std::collections::HashSet::from(["plc2"]));
        assert!(t.changed("plc", "temp", f64::NAN, Quality::Bad));
        assert!(!t.changed("plc2", "temp", 1.0, Quality::Good));
    }

    #[test]
    fn device_filter_selects_targets() {
        let hooks = vec![
            serde_json::json!({"id": 1, "url": "http://a", "device_filter": "\"all\""}),
            serde_json::json!({"id": 2, "url": "http://b", "device_filter": "[\"plc\"]"}),
            serde_json::json!({"id": 3, "url": "http://c", "device_filter": "other"}),
        ];
        let ids: Vec<i64> = webhook_targets(&hooks, &serde_json::json!({"device_id": "plc"}))
            .iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids, [1, 2]);
    }

    /// A local HTTP endpoint that counts the POSTs it receives.
    async fn counting_server() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let h = hits.clone();
        let app = axum::Router::new().route(
            "/hook",
            axum::routing::post(move || {
                let h = h.clone();
                async move {
                    h.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    "ok"
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/hook", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, hits)
    }

    #[tokio::test]
    async fn every_changed_tag_is_delivered() {
        let (url, hits) = counting_server().await;
        let storage = Storage::new_in_memory().unwrap();
        let queue = TagChangeQueue::start_with(
            WebhookSender::unchecked_for_tests(),
            storage,
            TAG_CHANGE_QUEUE_CAPACITY,
            4,
        );
        let hooks = vec![serde_json::json!({"id": 1, "url": url, "retry_count": 0})];
        let mut tracker = TagChangeTracker::default();
        // 200 tags change in one cycle: all are queued, recorded and sent.
        for i in 0..200 {
            let metric = format!("m{i}");
            assert!(tracker.changed("plc", &metric, 1.0, Quality::Good));
            assert!(queue.enqueue(
                &hooks,
                serde_json::json!({"device_id": "plc", "metric": metric})
            ));
            tracker.record("plc", &metric, 1.0, Quality::Good);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while hits.load(std::sync::atomic::Ordering::SeqCst) < 200 {
            assert!(
                std::time::Instant::now() < deadline,
                "only {hits:?} delivered"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn a_full_queue_refuses_instead_of_dropping_silently() {
        let storage = Storage::new_in_memory().unwrap();
        // No workers: nothing drains the queue.
        let queue = TagChangeQueue::start(storage, 3, 0);
        let hooks = vec![serde_json::json!({"id": 1, "url": "http://127.0.0.1:9/"})];
        let accepted = (0..10)
            .filter(|i| queue.enqueue(&hooks, serde_json::json!({"metric": i})))
            .count();
        assert_eq!(accepted, 3);
    }

    #[tokio::test]
    async fn alarm_webhooks_are_never_dropped() {
        let (url, hits) = counting_server().await;
        let storage = Storage::new_in_memory().unwrap();
        storage
            .create_outbound_webhook("a", &url, "[\"alarm\"]", "\"all\"", "{}", true, 0, 5000)
            .await
            .unwrap();
        // Far more alarms at once than there are alarm slots.
        for i in 0..(MAX_ALARM_DELIVERIES * 3) {
            dispatch_outbound_webhooks_with(
                &WebhookSender::unchecked_for_tests(),
                &storage,
                "alarm",
                &serde_json::json!({"alarm_id": i}),
            )
            .await;
        }
        let want = MAX_ALARM_DELIVERIES * 3;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while hits.load(std::sync::atomic::Ordering::SeqCst) < want {
            assert!(
                std::time::Instant::now() < deadline,
                "only {hits:?} of {want}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn real_deliveries_follow_the_outbound_policy() {
        // The default policy refuses this host: the webhook is not called
        // and its status says why.
        let (url, hits) = counting_server().await;
        let storage = Storage::new_in_memory().unwrap();
        let id = storage
            .create_outbound_webhook("a", &url, "[\"alarm\"]", "\"all\"", "{}", true, 0, 2000)
            .await
            .unwrap();
        let target = webhook_targets(
            &storage
                .get_outbound_webhooks_for_event("alarm")
                .await
                .unwrap(),
            &serde_json::json!({}),
        )
        .remove(0);
        deliver(
            &WebhookSender::policy(),
            &storage,
            &target,
            &serde_json::json!({}),
        )
        .await;
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
        let row = storage.get_outbound_webhook(id).await.unwrap().unwrap();
        let status = row
            .get("last_status")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(status.starts_with("refused"), "{row}");
    }
}
