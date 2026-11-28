use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast, watch};

use crate::models::{DeviceSnapshot, DeviceStatus};
use crate::quality::Quality;
use crate::ws_manager::WsManager;
use crate::ws_types::ServerMessage;

const SNAPSHOT_CHANNEL_CAPACITY: usize = 256;

/// A source timestamp further ahead of our clock than this is not believed:
/// the value is stamped with the time it arrived instead.
const MAX_SOURCE_CLOCK_AHEAD_MS: i64 = 5_000;

#[derive(Debug, Clone)]
pub struct DeviceCache {
    /// Each snapshot is shared with the [`Self::subscribe`] receivers that
    /// have not read it yet, and copied on write only while one still holds
    /// it: an update does not copy the whole device for every batch.
    devices: Arc<RwLock<HashMap<String, Arc<DeviceSnapshot>>>>,
    snapshot_tx: broadcast::Sender<Arc<DeviceSnapshot>>,
    trigger: watch::Sender<bool>,
    ws_manager: Option<Arc<WsManager>>,
    storage: Option<crate::storage::Storage>,
    /// device_id → acquisition protocol, registered by the poller manager when a
    /// poller starts; consulted on `update()` to tag each snapshot's `protocol`.
    protocols: Arc<RwLock<HashMap<String, String>>>,
    /// device_id → poll interval in ms, for stale detection.
    intervals: Arc<RwLock<HashMap<String, u64>>>,
}

/// One cached value with its quality and source time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    pub value: f64,
    pub quality: Quality,
    pub ts_ms: i64,
}

/// Set `map[key]` to `value`, allocating the key only when it is new.
fn set_in_place<V>(map: &mut HashMap<String, V>, key: &str, value: V) {
    match map.get_mut(key) {
        Some(slot) => *slot = value,
        None => {
            map.insert(key.to_owned(), value);
        }
    }
}

/// Mark the good values of `snap` that are older than the stale window.
fn mark_stale(snap: &mut DeviceSnapshot, interval_ms: Option<u64>, now_ms: i64) {
    if interval_ms.is_none() {
        return;
    }
    let stale: Vec<String> = snap
        .metrics
        .keys()
        .filter(|name| {
            let stored = snap.quality_of(name);
            crate::quality::effective(stored, snap.metric_ts(name), now_ms, interval_ms) != stored
        })
        .cloned()
        .collect();
    for name in stale {
        snap.quality.insert(name, Quality::UncertainStale);
    }
}

impl DeviceCache {
    pub fn new() -> Self {
        let (snapshot_tx, _) = broadcast::channel(SNAPSHOT_CHANNEL_CAPACITY);
        let (trigger, _) = watch::channel(false);
        Self {
            devices: Arc::new(RwLock::new(HashMap::new())),
            snapshot_tx,
            trigger,
            ws_manager: None,
            storage: None,
            protocols: Arc::new(RwLock::new(HashMap::new())),
            intervals: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Attach a WsManager so that device-status changes are broadcast over WebSocket.
    pub fn with_ws_manager(mut self, ws_manager: Arc<WsManager>) -> Self {
        self.ws_manager = Some(ws_manager);
        self
    }

    /// Attach a Storage so that device connect/disconnect events are written to the security log.
    pub fn with_storage(mut self, storage: crate::storage::Storage) -> Self {
        self.storage = Some(storage);
        self
    }

    /// Register the acquisition protocol for a device (called by the poller
    /// manager when a poller starts). Tags every subsequent snapshot so flow
    /// triggers can match on the real source instead of a hardcoded label.
    pub async fn register_protocol(&self, device_id: &str, protocol: &str) {
        self.protocols
            .write()
            .await
            .insert(device_id.to_string(), protocol.to_string());
        let mut devices = self.devices.write().await;
        if let Some(entry) = devices.get_mut(device_id) {
            Arc::make_mut(entry).protocol = Some(protocol.to_string());
        }
    }

    /// Replace a device's metrics with a full poll. A value's quality follows
    /// from the value: NaN (how drivers report a failed read) is bad.
    pub async fn update(&self, device_id: &str, metrics: HashMap<String, f64>, simulated: bool) {
        self.update_with_quality(device_id, metrics, HashMap::new(), simulated)
            .await;
    }

    /// Replace a device's metrics with a full poll, with the quality the driver
    /// reported for some of them (the others follow from their value).
    pub async fn update_with_quality(
        &self,
        device_id: &str,
        metrics: HashMap<String, f64>,
        quality: HashMap<String, Quality>,
        simulated: bool,
    ) {
        self.apply(device_id, metrics, quality, simulated, false)
            .await;
    }

    /// Update only the given metrics and keep the others: for sources that
    /// report changes rather than whole polls (OPC UA subscriptions), where an
    /// update that replaced the map would drop every tag that did not change.
    pub async fn merge_with_quality(
        &self,
        device_id: &str,
        metrics: HashMap<String, f64>,
        quality: HashMap<String, Quality>,
    ) {
        self.apply(device_id, metrics, quality, false, true).await;
    }

    async fn apply(
        &self,
        device_id: &str,
        metrics: HashMap<String, f64>,
        quality: HashMap<String, Quality>,
        simulated: bool,
        partial: bool,
    ) {
        self.write_device(device_id, simulated, |entry, ts_ms| {
            // A partial update keeps the metrics it does not carry, with their
            // own timestamp and quality (bad after a drop to offline, until the
            // device sends them again).
            if !partial {
                entry.metrics.clear();
                entry.quality.clear();
                entry.metric_ts_ms.clear();
            }
            for (name, value) in metrics {
                let q = quality
                    .get(&name)
                    .copied()
                    .unwrap_or_else(|| Quality::implied_by(value));
                if q.is_good() {
                    entry.quality.remove(&name);
                } else {
                    entry.quality.insert(name.clone(), q);
                }
                entry.metric_ts_ms.insert(name.clone(), ts_ms);
                entry.metrics.insert(name, value);
            }
        })
        .await;
    }

    /// Update the given metrics of a device and keep the others, like
    /// [`Self::merge_with_quality`], for a driver that keeps its tag names and
    /// reports changes as `(name, value, quality, source time in Unix ms)`.
    /// Tags already in the cache are updated in place, without allocating.
    /// The source time, when the device sends one, is the metric's timestamp
    /// (unless it is ahead of our clock by more than a few seconds).
    pub async fn merge_readings<'a>(
        &self,
        device_id: &str,
        readings: impl IntoIterator<Item = (&'a str, f64, Quality, Option<i64>)>,
    ) {
        self.write_device(device_id, false, |entry, now_ms| {
            for (name, value, quality, source_ms) in readings {
                let ts = source_ms
                    .filter(|t| *t <= now_ms + MAX_SOURCE_CLOCK_AHEAD_MS)
                    .unwrap_or(now_ms);
                set_in_place(&mut entry.metrics, name, value);
                set_in_place(&mut entry.metric_ts_ms, name, ts);
                if quality.is_good() {
                    if !entry.quality.is_empty() {
                        entry.quality.remove(name);
                    }
                } else {
                    set_in_place(&mut entry.quality, name, quality);
                }
            }
        })
        .await;
    }

    /// Mark a device online, let `write` change its metrics (given the time
    /// now in Unix ms), and tell the subscribers.
    async fn write_device(
        &self,
        device_id: &str,
        simulated: bool,
        write: impl FnOnce(&mut DeviceSnapshot, i64),
    ) {
        let (snapshot, was_offline) = {
            // Read the registered protocol (if any) before taking the device
            // write lock — lock order is always protocols → devices.
            let protocol = self.protocols.read().await.get(device_id).cloned();
            let mut devices = self.devices.write().await;
            let ts_ms = crate::timeutil::now_millis();

            let was_offline = devices
                .get(device_id)
                .map(|e| e.status == DeviceStatus::Offline)
                .unwrap_or(true);

            if !devices.contains_key(device_id) {
                devices.insert(
                    device_id.to_string(),
                    Arc::new(DeviceSnapshot {
                        device_id: device_id.to_string(),
                        status: DeviceStatus::Online,
                        timestamp: 0,
                        metrics: HashMap::new(),
                        simulated,
                        protocol: None,
                        ts_ms: 0,
                        quality: HashMap::new(),
                        metric_ts_ms: HashMap::new(),
                    }),
                );
            }
            let Some(shared) = devices.get_mut(device_id) else {
                return;
            };
            // Copies the snapshot only if a subscriber still holds it.
            let entry = Arc::make_mut(shared);
            entry.status = DeviceStatus::Online;
            entry.timestamp = ts_ms / 1000;
            entry.ts_ms = ts_ms;
            entry.simulated = simulated;
            if let Some(p) = protocol {
                entry.protocol = Some(p);
            }
            write(entry, ts_ms);

            (Arc::clone(shared), was_offline)
        };

        // Log device connect on transition to online
        if was_offline
            && crate::node_role::is_active()
            && let Some(storage) = &self.storage
        {
            let _ = storage
                .insert_security_event(&crate::models::CreateSecurityEvent {
                    event_type: "device_connected".into(),
                    severity: "info".into(),
                    source_ip: None,
                    user_agent: None,
                    username: None,
                    details: Some(format!("Device {} connected", device_id)),
                })
                .await;
        }

        // Only a receiver keeps the snapshot shared: without one, the next
        // update must not have to copy it.
        if self.snapshot_tx.receiver_count() > 0 {
            let _ = self.snapshot_tx.send(snapshot);
        } else {
            drop(snapshot);
        }
        let _ = self.trigger.send(true);

        // Broadcast device online status via WebSocket
        if let Some(ref ws) = self.ws_manager {
            ws.broadcast_device_status(ServerMessage::DeviceStatus {
                device_id: device_id.to_string(),
                online: true,
                latency_ms: None,
            });
        }
    }

    pub async fn set_status(&self, device_id: &str, status: DeviceStatus) {
        let old_status = {
            let mut devices = self.devices.write().await;
            let old = devices.get(device_id).map(|e| e.status.clone());
            if let Some(entry) = devices.get_mut(device_id) {
                let entry = Arc::make_mut(entry);
                entry.status = status.clone();
                // The last values stay readable for context, but nothing may
                // act on them as if they were current.
                if status == DeviceStatus::Offline {
                    for name in entry.metrics.keys() {
                        entry.quality.insert(name.clone(), Quality::BadCommFailure);
                    }
                }
            }
            old
        };

        // Log device connect/disconnect transitions
        if crate::node_role::is_active()
            && let Some(storage) = &self.storage
        {
            match (&old_status, &status) {
                (Some(DeviceStatus::Offline), DeviceStatus::Online)
                | (None, DeviceStatus::Online) => {
                    let _ = storage
                        .insert_security_event(&crate::models::CreateSecurityEvent {
                            event_type: "device_connected".into(),
                            severity: "info".into(),
                            source_ip: None,
                            user_agent: None,
                            username: None,
                            details: Some(format!("Device {} connected", device_id)),
                        })
                        .await;
                }
                (Some(DeviceStatus::Online), DeviceStatus::Offline) => {
                    let _ = storage
                        .insert_security_event(&crate::models::CreateSecurityEvent {
                            event_type: "device_disconnected".into(),
                            severity: "warning".into(),
                            source_ip: None,
                            user_agent: None,
                            username: None,
                            details: Some(format!("Device {} disconnected", device_id)),
                        })
                        .await;
                }
                _ => {}
            }
        }

        // Broadcast device status change via WebSocket
        if let Some(ref ws) = self.ws_manager {
            ws.broadcast_device_status(ServerMessage::DeviceStatus {
                device_id: device_id.to_string(),
                online: status == DeviceStatus::Online,
                latency_ms: None,
            });
        }
    }

    /// Every device's snapshot, with values that stopped updating marked
    /// stale (see [`crate::quality::effective`]).
    pub async fn get_all(&self) -> Vec<DeviceSnapshot> {
        self.get_all_at(crate::timeutil::now_millis()).await
    }

    /// [`Self::get_all`] as seen at `now_ms`.
    pub async fn get_all_at(&self, now_ms: i64) -> Vec<DeviceSnapshot> {
        let intervals = self.intervals.read().await;
        let devices = self.devices.read().await;
        devices
            .values()
            .map(|snap| {
                let mut snap = DeviceSnapshot::clone(snap);
                let interval = intervals.get(&snap.device_id).copied();
                mark_stale(&mut snap, interval, now_ms);
                snap
            })
            .collect()
    }

    /// Register how often a device is polled, so values it stops refreshing
    /// can be told apart from current ones. Report-by-exception sources do not
    /// register: an unchanged value is not re-sent, so age means nothing there.
    pub async fn register_poll_interval(&self, device_id: &str, interval_ms: u64) {
        self.intervals
            .write()
            .await
            .insert(device_id.to_string(), interval_ms);
    }

    pub async fn get_statuses(&self) -> HashMap<String, String> {
        let devices = self.devices.read().await;
        devices
            .iter()
            .map(|(id, snap)| {
                let status_str = match snap.status {
                    DeviceStatus::Online => "online",
                    DeviceStatus::Offline => "offline",
                };
                (id.clone(), status_str.to_string())
            })
            .collect()
    }

    pub async fn is_device_online(&self, device_id: &str) -> bool {
        let devices = self.devices.read().await;
        devices
            .get(device_id)
            .is_some_and(|s| s.status == DeviceStatus::Online)
    }

    pub async fn get_metric_value(&self, key: &str) -> Option<f64> {
        let devices = self.devices.read().await;
        for snapshot in devices.values() {
            if let Some(v) = snapshot.metrics.get(key) {
                return Some(*v);
            }
        }
        None
    }

    /// The cached value of `metric` on one device, `None` when the device or the metric is not there.
    pub async fn get_device_metric(&self, device_id: &str, metric: &str) -> Option<f64> {
        self.devices
            .read()
            .await
            .get(device_id)
            .and_then(|snapshot| snapshot.metrics.get(metric).copied())
    }

    /// One metric of one device as it reads now: value, quality (stale applied)
    /// and source timestamp in Unix ms.
    pub async fn get_reading(&self, device_id: &str, metric: &str) -> Option<Reading> {
        let interval = self.intervals.read().await.get(device_id).copied();
        let devices = self.devices.read().await;
        let snap = devices.get(device_id)?;
        let value = *snap.metrics.get(metric)?;
        let ts_ms = snap.metric_ts(metric);
        let quality = crate::quality::effective(
            snap.quality_of(metric),
            ts_ms,
            crate::timeutil::now_millis(),
            interval,
        );
        Some(Reading {
            value,
            quality,
            ts_ms,
        })
    }

    /// The poll interval registered for a device, if it is polled.
    pub async fn poll_interval(&self, device_id: &str) -> Option<u64> {
        self.intervals.read().await.get(device_id).copied()
    }

    /// Drop a device from the cache entirely. Called when a device is deleted:
    /// without this the device keeps appearing as online in `/api/v1/live`
    /// until process restart.
    pub async fn remove(&self, device_id: &str) {
        self.protocols.write().await.remove(device_id);
        self.intervals.write().await.remove(device_id);
        self.devices.write().await.remove(device_id);
    }

    /// Every device update, as the device's whole snapshot after it. Holding
    /// a received snapshot makes the next update of that device copy it, so
    /// receivers should drop it once read.
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<DeviceSnapshot>> {
        self.snapshot_tx.subscribe()
    }

    pub fn trigger_receiver(&self) -> watch::Receiver<bool> {
        self.trigger.subscribe()
    }

    /// Broadcast an HA status message (built by the add-on that owns HA) via WebSocket.
    pub async fn broadcast_ha_status(&self, message: ServerMessage) {
        if let Some(ref ws) = self.ws_manager {
            ws.broadcast_ha_status(message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_update_and_get_all() {
        let cache = DeviceCache::new();

        let metrics = HashMap::from([("voltage".to_string(), 220.5), ("current".to_string(), 5.2)]);

        cache.update("plts_1", metrics, false).await;

        let all = cache.get_all().await;
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].device_id, "plts_1");
        assert_eq!(all[0].status, DeviceStatus::Online);
        assert_eq!(all[0].metrics.get("voltage"), Some(&220.5));
        assert_eq!(all[0].metrics.get("current"), Some(&5.2));
    }

    #[tokio::test]
    async fn test_multiple_devices() {
        let cache = DeviceCache::new();

        let m1 = HashMap::from([("voltage".into(), 220.0)]);
        let m2 = HashMap::from([("voltage".into(), 380.0)]);
        cache.update("dev1", m1, false).await;
        cache.update("dev2", m2, false).await;

        let all = cache.get_all().await;
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_update_overwrites() {
        let cache = DeviceCache::new();

        let m1 = HashMap::from([("voltage".into(), 220.0)]);
        let m2 = HashMap::from([("voltage".into(), 225.0)]);
        cache.update("dev1", m1, false).await;
        cache.update("dev1", m2, false).await;

        let all = cache.get_all().await;
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].metrics.get("voltage"), Some(&225.0));
    }

    #[tokio::test]
    async fn test_set_status_offline() {
        let cache = DeviceCache::new();

        let metrics = HashMap::from([("voltage".into(), 220.0)]);
        cache.update("dev1", metrics, false).await;
        cache.set_status("dev1", DeviceStatus::Offline).await;

        let all = cache.get_all().await;
        assert_eq!(all[0].status, DeviceStatus::Offline);

        let statuses = cache.get_statuses().await;
        assert_eq!(statuses.get("dev1"), Some(&"offline".to_string()));
    }

    #[tokio::test]
    async fn test_get_statuses() {
        let cache = DeviceCache::new();

        let m1 = HashMap::from([("voltage".into(), 220.0)]);
        let m2 = HashMap::from([("voltage".into(), 380.0)]);
        cache.update("dev1", m1, false).await;
        cache.update("dev2", m2, false).await;
        cache.set_status("dev2", DeviceStatus::Offline).await;

        let statuses = cache.get_statuses().await;
        assert_eq!(statuses.get("dev1"), Some(&"online".to_string()));
        assert_eq!(statuses.get("dev2"), Some(&"offline".to_string()));
    }

    #[tokio::test]
    async fn test_set_status_nonexistent() {
        let cache = DeviceCache::new();
        cache.set_status("ghost", DeviceStatus::Offline).await;

        let all = cache.get_all().await;
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn test_trigger_fires_on_update() {
        let cache = DeviceCache::new();
        let mut rx = cache.trigger_receiver();

        let metrics = HashMap::from([("voltage".into(), 220.0)]);
        cache.update("dev1", metrics, false).await;

        tokio::time::timeout(std::time::Duration::from_millis(50), rx.changed())
            .await
            .expect("trigger should fire on update")
            .expect("should succeed");
    }

    #[tokio::test]
    async fn test_simulated_flag_defaults_false() {
        let cache = DeviceCache::new();

        let metrics = HashMap::from([("voltage".into(), 220.0)]);
        cache.update("dev1", metrics, false).await;

        let all = cache.get_all().await;
        assert_eq!(all.len(), 1);
        assert!(!all[0].simulated);
    }

    #[tokio::test]
    async fn test_simulated_flag_set_true() {
        let cache = DeviceCache::new();

        let metrics = HashMap::from([("voltage".into(), 220.0)]);
        cache.update("dev1", metrics, true).await;

        let all = cache.get_all().await;
        assert_eq!(all.len(), 1);
        assert!(all[0].simulated);
    }

    #[tokio::test]
    async fn test_simulated_flag_toggles() {
        let cache = DeviceCache::new();

        let m1 = HashMap::from([("voltage".into(), 220.0)]);
        let m2 = HashMap::from([("voltage".into(), 225.0)]);

        cache.update("dev1", m1, false).await;
        assert!(!cache.get_all().await[0].simulated);

        cache.update("dev1", m2.clone(), true).await;
        assert!(cache.get_all().await[0].simulated);

        cache.update("dev1", m2, false).await;
        assert!(!cache.get_all().await[0].simulated);
    }

    #[tokio::test]
    async fn nan_is_bad_and_driver_quality_is_kept() {
        let cache = DeviceCache::new();
        cache
            .update_with_quality(
                "dev1",
                HashMap::from([
                    ("ok".into(), 1.0),
                    ("nan".into(), f64::NAN),
                    ("item".into(), f64::NAN),
                ]),
                HashMap::from([("item".into(), Quality::BadDeviceError)]),
                false,
            )
            .await;
        let snap = &cache.get_all().await[0];
        assert_eq!(snap.quality_of("ok"), Quality::Good);
        assert_eq!(snap.quality_of("nan"), Quality::Bad);
        assert_eq!(snap.quality_of("item"), Quality::BadDeviceError);
        assert!(
            !snap.quality.contains_key("ok"),
            "good values are not listed"
        );
        assert!(snap.metric_ts("ok") > 0);
    }

    #[tokio::test]
    async fn offline_marks_every_value_comm_failure_until_read_again() {
        let cache = DeviceCache::new();
        cache
            .update("dev1", HashMap::from([("v".into(), 220.0)]), false)
            .await;
        cache.set_status("dev1", DeviceStatus::Offline).await;
        let snap = &cache.get_all().await[0];
        assert_eq!(snap.metrics["v"], 220.0, "the last value stays for context");
        assert_eq!(snap.quality_of("v"), Quality::BadCommFailure);
        let r = cache.get_reading("dev1", "v").await.unwrap();
        assert_eq!(r.quality, Quality::BadCommFailure);

        cache
            .update("dev1", HashMap::from([("v".into(), 221.0)]), false)
            .await;
        assert_eq!(cache.get_all().await[0].quality_of("v"), Quality::Good);
    }

    #[tokio::test]
    async fn values_go_stale_after_three_poll_intervals() {
        let cache = DeviceCache::new();
        cache.register_poll_interval("dev1", 2_000).await;
        cache
            .update("dev1", HashMap::from([("v".into(), 1.0)]), false)
            .await;
        let read_at = cache.get_all().await[0].metric_ts("v");
        let at = |ms| cache.get_all_at(read_at + ms);
        assert_eq!(at(6_000).await[0].quality_of("v"), Quality::Good);
        assert_eq!(at(6_001).await[0].quality_of("v"), Quality::UncertainStale);
    }

    #[tokio::test]
    async fn without_a_poll_interval_nothing_goes_stale() {
        let cache = DeviceCache::new();
        cache
            .update("dev1", HashMap::from([("v".into(), 1.0)]), false)
            .await;
        let read_at = cache.get_all().await[0].metric_ts("v");
        let snaps = cache.get_all_at(read_at + 3_600_000).await;
        assert_eq!(snaps[0].quality_of("v"), Quality::Good);
    }

    #[tokio::test]
    async fn a_merge_keeps_the_metrics_it_does_not_carry() {
        let cache = DeviceCache::new();
        cache
            .merge_with_quality(
                "ua",
                HashMap::from([("a".into(), 1.0), ("b".into(), 2.0)]),
                HashMap::new(),
            )
            .await;
        cache
            .merge_with_quality(
                "ua",
                HashMap::from([("a".into(), f64::NAN)]),
                HashMap::from([("a".into(), Quality::BadDeviceError)]),
            )
            .await;
        let snap = &cache.get_all().await[0];
        assert_eq!(snap.metrics["b"], 2.0);
        assert_eq!(snap.quality_of("a"), Quality::BadDeviceError);
        assert_eq!(snap.quality_of("b"), Quality::Good);
        // A full update replaces the map, as polls always have.
        cache
            .update("ua", HashMap::from([("c".into(), 3.0)]), false)
            .await;
        let snap = &cache.get_all().await[0];
        assert!(!snap.metrics.contains_key("b"));
        assert!(snap.quality.is_empty());
    }

    #[tokio::test]
    async fn merge_readings_keeps_other_tags_quality_and_source_time() {
        let cache = DeviceCache::new();
        let now = crate::timeutil::now_millis();
        cache
            .merge_readings(
                "ua",
                [
                    ("a", 1.0, Quality::Good, Some(now - 60_000)),
                    ("b", f64::NAN, Quality::BadDeviceError, None),
                ],
            )
            .await;
        cache
            .merge_readings(
                "ua",
                [
                    ("b", 2.0, Quality::Good, Some(now)),
                    // A clock far ahead is not believed.
                    ("c", 3.0, Quality::Uncertain, Some(now + 3_600_000)),
                ],
            )
            .await;
        let snap = &cache.get_all().await[0];
        assert_eq!(snap.status, DeviceStatus::Online);
        assert_eq!(snap.metrics["a"], 1.0);
        assert_eq!(snap.metric_ts("a"), now - 60_000, "the source time");
        assert_eq!(snap.metrics["b"], 2.0);
        assert_eq!(snap.quality_of("b"), Quality::Good, "good again");
        assert_eq!(snap.quality_of("c"), Quality::Uncertain);
        assert!(snap.metric_ts("c") < now + 60_000, "arrival time instead");
    }

    /// A subscriber gets each update's snapshot as it was; holding it does not
    /// see later updates (the cache copies on write), and receivers that have
    /// read it no longer hold it.
    #[tokio::test]
    async fn subscribers_get_each_snapshot_as_it_was() {
        let cache = DeviceCache::new();
        let mut rx = cache.subscribe();
        cache
            .merge_readings("ua", [("a", 1.0, Quality::Good, None)])
            .await;
        let first = rx.recv().await.unwrap();
        cache
            .merge_readings("ua", [("a", 2.0, Quality::Good, None)])
            .await;
        let second = rx.recv().await.unwrap();
        assert_eq!(first.metrics["a"], 1.0, "held snapshot unchanged");
        assert_eq!(second.metrics["a"], 2.0);
        drop(first);
        // Only the cache and `second` hold the current snapshot now; the
        // channel let go of it once read.
        let devices = cache.devices.read().await;
        assert_eq!(Arc::strong_count(&devices["ua"]), 2);
    }
}
