use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast, watch};

use crate::models::{DeviceSnapshot, DeviceStatus};
use crate::ws_manager::WsManager;
use crate::ws_types::ServerMessage;

const SNAPSHOT_CHANNEL_CAPACITY: usize = 256;

#[derive(Debug, Clone)]
pub struct DeviceCache {
    devices: Arc<RwLock<HashMap<String, DeviceSnapshot>>>,
    snapshot_tx: broadcast::Sender<DeviceSnapshot>,
    trigger: watch::Sender<bool>,
    ws_manager: Option<Arc<WsManager>>,
    storage: Option<crate::storage::Storage>,
    /// device_id → acquisition protocol, registered by the poller manager when a
    /// poller starts; consulted on `update()` to tag each snapshot's `protocol`.
    protocols: Arc<RwLock<HashMap<String, String>>>,
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
            entry.protocol = Some(protocol.to_string());
        }
    }

    pub async fn update(&self, device_id: &str, metrics: HashMap<String, f64>, simulated: bool) {
        let (snapshot, was_offline) = {
            // Read the registered protocol (if any) before taking the device
            // write lock — lock order is always protocols → devices.
            let protocol = self.protocols.read().await.get(device_id).cloned();
            let mut devices = self.devices.write().await;
            let timestamp = crate::timeutil::now_secs();

            let was_offline = devices
                .get(device_id)
                .map(|e| e.status == DeviceStatus::Offline)
                .unwrap_or(true);

            let snapshot = devices
                .entry(device_id.to_string())
                .and_modify(|e| {
                    e.status = DeviceStatus::Online;
                    e.timestamp = timestamp;
                    e.metrics = metrics.clone();
                    e.simulated = simulated;
                    if let Some(p) = &protocol {
                        e.protocol = Some(p.clone());
                    }
                })
                .or_insert_with(|| DeviceSnapshot {
                    device_id: device_id.to_string(),
                    status: DeviceStatus::Online,
                    timestamp,
                    metrics,
                    simulated,
                    protocol,
                })
                .clone();

            (snapshot, was_offline)
        };

        // Log device connect on transition to online
        if was_offline && let Some(ref storage) = self.storage {
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

        let _ = self.snapshot_tx.send(snapshot);
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
                entry.status = status.clone();
            }
            old
        };

        // Log device connect/disconnect transitions
        if let Some(ref storage) = self.storage {
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

    pub async fn get_all(&self) -> Vec<DeviceSnapshot> {
        let devices = self.devices.read().await;
        devices.values().cloned().collect()
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

    /// Update a single metric value within an existing device snapshot and broadcast the change.
    /// If the device or metric doesn't exist yet, this is a no-op.
    pub async fn set_metric_value(&self, device_id: &str, metric_key: &str, value: f64) {
        let snapshot = {
            let mut devices = self.devices.write().await;
            let Some(entry) = devices.get_mut(device_id) else {
                return;
            };
            entry.metrics.insert(metric_key.to_string(), value);
            entry.timestamp = crate::timeutil::now_secs();
            entry.clone()
        };
        let _ = self.snapshot_tx.send(snapshot);
        let _ = self.trigger.send(true);
    }

    /// Drop a device from the cache entirely. Called when a device is deleted:
    /// without this the device keeps appearing as online in `/api/v1/live`
    /// until process restart.
    pub async fn remove(&self, device_id: &str) {
        self.protocols.write().await.remove(device_id);
        self.devices.write().await.remove(device_id);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DeviceSnapshot> {
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
}
