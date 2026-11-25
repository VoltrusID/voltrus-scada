//! The Voltrus SCADA phone app: site settings for paired phones and alarm
//! push dispatch.
//!
//! Phones pair with a site (storage/mobile.rs, api/mobile.rs) and then use the
//! REST API with a device token. Pushes go out through the Voltrus push relay,
//! which an add-on edition registers as a [`MobilePushSender`]; core has no
//! relay connection of its own and never calls Voltrus servers. Without a
//! sender, or below the Integrator tier, the dispatcher sends nothing and the
//! app still works read-only on the site's own network.
//!
//! [`MobilePushSender`]: crate::edition::MobilePushSender

use std::sync::{Arc, RwLock};

use crate::edition::{MobileAlarmPush, MobilePushTarget};
use crate::license::LicenseStatus;
use crate::storage::{MobileDevice, Storage};
use crate::ws_manager::WsManager;
use crate::ws_types::ServerMessage;

/// Settings keys (the `settings` table).
pub const SITE_NAME_KEY: &str = "mobile_site_name";
pub const PUSH_DETAILS_KEY: &str = "mobile_push_details";
pub const PINNED_TAGS_KEY: &str = "mobile_pinned_tags";
pub const PUBLIC_URLS_KEY: &str = "mobile_public_urls";

/// At most this many pinned tags are shown on a phone.
pub const MAX_PINNED_TAGS: usize = 24;
/// At most this many extra addresses go into a pairing QR code.
pub const MAX_PUBLIC_URLS: usize = 4;

/// Pushes for new alarms per minute before the rest of the minute is folded
/// into one summary push.
pub const BURST_PER_MINUTE: u32 = 20;
/// Pushes per hour per site, summaries and state changes included. The push
/// relay enforces the same cap.
pub const PUSHES_PER_HOUR: u32 = 600;

/// The priorities a phone may filter on, lowest first.
pub const PRIORITIES: [&str; 3] = ["info", "warning", "critical"];

/// Rank of an alarm priority (`info` < `warning` < `critical`). Anything
/// unknown ranks as `warning`, as the alarm table stores it.
pub fn priority_rank(priority: &str) -> usize {
    PRIORITIES
        .iter()
        .position(|p| p.eq_ignore_ascii_case(priority))
        .unwrap_or(1)
}

/// What an admin sets for the phone app.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct MobileSettings {
    /// Name of the site on the phone. Empty: the licence's site name, else
    /// the white-label title, else "Voltrus SCADA".
    #[serde(default)]
    pub site_name: String,
    /// Put the alarm text in the notification. Off: the notification says
    /// only that the site has a new alarm, and the app fetches the rest.
    #[serde(default = "default_true")]
    pub push_details: bool,
    /// Tag keys shown with live values on the phone's site screen, in order.
    #[serde(default)]
    pub pinned_tags: Vec<String>,
    /// Extra addresses of the site put in the pairing QR code (for example
    /// the address on the plant VPN), tried before the relay.
    #[serde(default)]
    pub public_urls: Vec<String>,
}

fn default_true() -> bool {
    true
}

impl Default for MobileSettings {
    fn default() -> Self {
        Self {
            site_name: String::new(),
            push_details: true,
            pinned_tags: Vec::new(),
            public_urls: Vec::new(),
        }
    }
}

impl MobileSettings {
    pub async fn load(storage: &Storage) -> anyhow::Result<Self> {
        let rows = storage
            .get_settings(&[
                SITE_NAME_KEY,
                PUSH_DETAILS_KEY,
                PINNED_TAGS_KEY,
                PUBLIC_URLS_KEY,
            ])
            .await?;
        let get = |key: &str| rows.iter().find(|s| s.key == key).map(|s| s.value.as_str());
        let list = |key: &str| -> Vec<String> {
            get(key)
                .and_then(|v| serde_json::from_str(v).ok())
                .unwrap_or_default()
        };
        Ok(Self {
            site_name: get(SITE_NAME_KEY).unwrap_or_default().to_string(),
            push_details: get(PUSH_DETAILS_KEY) != Some("false"),
            pinned_tags: list(PINNED_TAGS_KEY),
            public_urls: list(PUBLIC_URLS_KEY),
        })
    }

    pub async fn save(&self, storage: &Storage) -> anyhow::Result<()> {
        storage
            .upsert_setting(SITE_NAME_KEY, self.site_name.trim())
            .await?;
        storage
            .upsert_setting(
                PUSH_DETAILS_KEY,
                if self.push_details { "true" } else { "false" },
            )
            .await?;
        storage
            .upsert_setting(PINNED_TAGS_KEY, &serde_json::to_string(&self.pinned_tags)?)
            .await?;
        storage
            .upsert_setting(PUBLIC_URLS_KEY, &serde_json::to_string(&self.public_urls)?)
            .await?;
        Ok(())
    }

    /// Check settings an admin is saving; returns them trimmed.
    pub fn validated(mut self) -> Result<Self, String> {
        self.site_name = self.site_name.trim().to_string();
        if self.site_name.chars().count() > 80 {
            return Err("site_name must be at most 80 characters".to_string());
        }
        let mut seen = std::collections::HashSet::new();
        self.pinned_tags = self
            .pinned_tags
            .into_iter()
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty() && seen.insert(k.clone()))
            .collect();
        if self.pinned_tags.len() > MAX_PINNED_TAGS {
            return Err(format!("at most {MAX_PINNED_TAGS} pinned tags"));
        }
        self.public_urls = self
            .public_urls
            .into_iter()
            .map(|u| u.trim().trim_end_matches('/').to_string())
            .filter(|u| !u.is_empty())
            .collect();
        if self.public_urls.len() > MAX_PUBLIC_URLS {
            return Err(format!("at most {MAX_PUBLIC_URLS} addresses"));
        }
        for url in &self.public_urls {
            let ok = reqwest::Url::parse(url)
                .ok()
                .is_some_and(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some());
            if !ok {
                return Err(format!("{url:?} is not an http(s) address"));
            }
        }
        Ok(self)
    }
}

/// The site's name on the phone: the admin's choice, else the licence's site
/// name, else the white-label title (when white-label is licensed), else
/// "Voltrus SCADA".
pub async fn site_name(
    storage: &Storage,
    settings: &MobileSettings,
    license: &LicenseStatus,
) -> String {
    if !settings.site_name.is_empty() {
        return settings.site_name.clone();
    }
    if let LicenseStatus::Valid {
        site: Some(site), ..
    } = license
        && !site.name.trim().is_empty()
    {
        return site.name.trim().to_string();
    }
    if license.white_label()
        && let Ok(rows) = storage.get_settings(&["brand_title"]).await
        && let Some(title) = rows
            .first()
            .map(|s| s.value.trim())
            .filter(|t| !t.is_empty())
    {
        return title.to_string();
    }
    "Voltrus SCADA".to_string()
}

/// Spreads pushes out: [`BURST_PER_MINUTE`] new alarms a minute, then one
/// summary for the rest of the minute; [`PUSHES_PER_HOUR`] pushes an hour in
/// all. Windows are fixed (minute and hour since the first push).
#[derive(Debug, Default)]
pub struct PushBudget {
    minute_start: i64,
    minute_count: u32,
    held_back: u32,
    hour_start: i64,
    hour_count: u32,
}

impl PushBudget {
    fn roll(&mut self, now: i64) {
        if now - self.hour_start >= 3600 {
            self.hour_start = now;
            self.hour_count = 0;
        }
        if now - self.minute_start >= 60 {
            self.minute_start = now;
            self.minute_count = 0;
        }
    }

    /// Whether a push for a new alarm may go out now. One that may not is
    /// counted for the next summary.
    pub fn admit_raised(&mut self, now: i64) -> bool {
        self.roll(now);
        if self.hour_count >= PUSHES_PER_HOUR || self.minute_count >= BURST_PER_MINUTE {
            self.held_back += 1;
            return false;
        }
        self.minute_count += 1;
        self.hour_count += 1;
        true
    }

    /// Whether a state change (acknowledged, resolved…) may go out now.
    /// Changes are never summarised; past the hourly cap they are dropped.
    pub fn admit_update(&mut self, now: i64) -> bool {
        self.roll(now);
        if self.hour_count >= PUSHES_PER_HOUR {
            return false;
        }
        self.hour_count += 1;
        true
    }

    /// Alarms held back once their minute is over and the hour has room:
    /// send one summary for them. `None` while there is nothing to send yet.
    pub fn take_summary(&mut self, now: i64) -> Option<u32> {
        if self.held_back == 0 || now - self.minute_start < 60 {
            return None;
        }
        self.roll(now);
        if self.hour_count >= PUSHES_PER_HOUR {
            return None;
        }
        self.hour_count += 1;
        Some(std::mem::take(&mut self.held_back))
    }
}

/// Phones `alarm_priority` is pushed to: a push token, a user who still exists
/// and is not disabled, and the phone's lowest priority at or below the alarm's.
pub async fn push_targets(
    storage: &Storage,
    alarm_priority: Option<&str>,
) -> anyhow::Result<Vec<MobilePushTarget>> {
    let devices = storage.mobile_push_devices().await?;
    let mut users: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    let mut targets = Vec::new();
    for device in devices {
        if let Some(priority) = alarm_priority
            && priority_rank(priority) < priority_rank(&device.min_priority)
        {
            continue;
        }
        let allowed = match users.get(&device.username) {
            Some(allowed) => *allowed,
            None => {
                let allowed = storage
                    .get_user_by_username(&device.username)
                    .await
                    .ok()
                    .flatten()
                    .is_some_and(|u| !u.disabled);
                users.insert(device.username.clone(), allowed);
                allowed
            }
        };
        if !allowed {
            continue;
        }
        let MobileDevice {
            id,
            platform,
            push_token,
            lang,
            ..
        } = device;
        if let Some(push_token) = push_token {
            targets.push(MobilePushTarget {
                device_id: id,
                platform,
                push_token,
                lang,
            });
        }
    }
    Ok(targets)
}

/// Push alarm events to paired phones as they are broadcast. Does nothing
/// while no [`MobilePushSender`](crate::edition::MobilePushSender) is
/// registered, the tier lacks `mobile_push`, or this node is a standby.
pub fn spawn_dispatcher(
    storage: Storage,
    ws_manager: Arc<WsManager>,
    license_status: Arc<RwLock<LicenseStatus>>,
) {
    let mut events = ws_manager.subscribe_alarm_events();
    tokio::spawn(async move {
        let mut budget = PushBudget::default();
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(10));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let event = tokio::select! {
                event = events.recv() => match event {
                    Ok(event) => Some(event),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("Mobile push: {n} alarm events skipped (lagging)");
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                _ = tick.tick() => None,
            };
            let license = license_status
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            if crate::edition::mobile_push_sender().is_none()
                || !license.tier().mobile_push()
                || !crate::node_role::is_active()
            {
                continue;
            }
            let now = crate::timeutil::now_secs();
            let push = match event {
                None => match budget.take_summary(now) {
                    Some(held_back) => summary_push(&storage, &license, held_back, now).await,
                    None => continue,
                },
                Some(ServerMessage::Alarm { alarm_id, .. }) => {
                    if !budget.admit_raised(now) {
                        continue;
                    }
                    alarm_push(&storage, &license, alarm_id, "raised", None, now).await
                }
                Some(ServerMessage::AlarmUpdate {
                    alarm_id,
                    event,
                    actor,
                    ..
                }) => {
                    if !budget.admit_update(now) {
                        continue;
                    }
                    alarm_push(&storage, &license, alarm_id, &event, actor, now).await
                }
                Some(_) => continue,
            };
            match push {
                Ok(Some(push)) => send(&storage, push).await,
                Ok(None) => {}
                Err(e) => tracing::warn!("Mobile push: {e:#}"),
            }
        }
    });
}

async fn alarm_push(
    storage: &Storage,
    license: &LicenseStatus,
    alarm_id: i64,
    event: &str,
    actor: Option<String>,
    now: i64,
) -> anyhow::Result<Option<MobileAlarmPush>> {
    let Some(alarm) = storage.get_alarm(alarm_id).await? else {
        return Ok(None);
    };
    let targets = push_targets(storage, Some(&alarm.priority)).await?;
    if targets.is_empty() {
        return Ok(None);
    }
    let settings = MobileSettings::load(storage).await?;
    let source = storage
        .list_data_sources(None)
        .await?
        .into_iter()
        .find(|s| s.key == alarm.source_key);
    Ok(Some(MobileAlarmPush {
        event: event.to_string(),
        alarm_id,
        site_name: site_name(storage, &settings, license).await,
        source_name: source
            .as_ref()
            .map(|s| s.name.clone())
            .unwrap_or_else(|| alarm.source_key.clone()),
        unit: source.map(|s| s.unit).unwrap_or_default(),
        source_key: alarm.source_key,
        severity: alarm.severity,
        priority: alarm.priority,
        message: alarm.message,
        value: alarm.value,
        actor,
        ts: now,
        details: settings.push_details,
        held_back: 0,
        targets,
    }))
}

async fn summary_push(
    storage: &Storage,
    license: &LicenseStatus,
    held_back: u32,
    now: i64,
) -> anyhow::Result<Option<MobileAlarmPush>> {
    let targets = push_targets(storage, None).await?;
    if targets.is_empty() {
        return Ok(None);
    }
    let settings = MobileSettings::load(storage).await?;
    Ok(Some(MobileAlarmPush {
        event: "summary".to_string(),
        alarm_id: 0,
        site_name: site_name(storage, &settings, license).await,
        source_key: String::new(),
        source_name: String::new(),
        severity: "warning".to_string(),
        priority: "warning".to_string(),
        message: String::new(),
        value: 0.0,
        unit: String::new(),
        actor: None,
        ts: now,
        details: settings.push_details,
        held_back,
        targets,
    }))
}

async fn send(storage: &Storage, push: MobileAlarmPush) {
    let Some(sender) = crate::edition::mobile_push_sender() else {
        return;
    };
    match sender.send(&push).await {
        Ok(deliveries) => {
            for d in deliveries {
                if d.unregistered
                    && let Some(target) = push.targets.iter().find(|t| t.device_id == d.device_id)
                {
                    let _ = storage
                        .clear_mobile_push_token(d.device_id, &target.push_token)
                        .await;
                }
                if let Some(err) = d.error {
                    tracing::warn!(
                        "Mobile push for alarm {} to device {}: {err}",
                        push.alarm_id,
                        d.device_id
                    );
                }
            }
        }
        Err(e) => tracing::warn!("Mobile push for alarm {}: {e:#}", push.alarm_id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priorities_rank_info_warning_critical() {
        assert!(priority_rank("info") < priority_rank("warning"));
        assert!(priority_rank("warning") < priority_rank("critical"));
        assert_eq!(priority_rank("CRITICAL"), priority_rank("critical"));
        assert_eq!(priority_rank("something"), priority_rank("warning"));
    }

    #[test]
    fn a_burst_is_folded_into_one_summary() {
        let mut b = PushBudget::default();
        let t0 = 1_000_000;
        for _ in 0..BURST_PER_MINUTE {
            assert!(b.admit_raised(t0));
        }
        assert!(!b.admit_raised(t0 + 1));
        assert!(!b.admit_raised(t0 + 2));
        // Not before the minute is over.
        assert_eq!(b.take_summary(t0 + 30), None);
        assert_eq!(b.take_summary(t0 + 60), Some(2));
        assert_eq!(b.take_summary(t0 + 70), None);
        // A new minute admits again.
        assert!(b.admit_raised(t0 + 61));
    }

    #[test]
    fn the_hourly_cap_holds_everything() {
        let mut b = PushBudget::default();
        let mut t = 5_000_000;
        let mut sent = 0;
        while sent < PUSHES_PER_HOUR {
            if b.admit_raised(t) {
                sent += 1;
            } else {
                t += 60;
            }
        }
        assert!(!b.admit_update(t));
        assert!(!b.admit_raised(t + 60));
        assert_eq!(b.take_summary(t + 120), None);
        // The next hour starts over, with the summary first.
        let next_hour = b.hour_start + 3600;
        assert!(b.take_summary(next_hour).is_some());
        assert!(b.admit_update(next_hour));
    }

    #[test]
    fn settings_are_trimmed_and_checked() {
        let s = MobileSettings {
            site_name: "  Pabrik A ".into(),
            push_details: false,
            pinned_tags: vec![" a ".into(), "a".into(), "".into(), "b".into()],
            public_urls: vec!["http://10.0.0.5:3000/".into()],
        }
        .validated()
        .unwrap();
        assert_eq!(s.site_name, "Pabrik A");
        assert_eq!(s.pinned_tags, ["a", "b"]);
        assert_eq!(s.public_urls, ["http://10.0.0.5:3000"]);

        let bad = MobileSettings {
            public_urls: vec!["ftp://x".into()],
            ..Default::default()
        };
        assert!(bad.validated().is_err());
        let many = MobileSettings {
            pinned_tags: (0..=MAX_PINNED_TAGS).map(|i| i.to_string()).collect(),
            ..Default::default()
        };
        assert!(many.validated().is_err());
    }
}
