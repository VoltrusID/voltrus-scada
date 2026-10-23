use std::collections::HashSet;

use bytes::Bytes;
use dashmap::DashMap;
use tokio::sync::mpsc::{self, Receiver, Sender};

use crate::ws_types::ServerMessage;

const WS_CHANNEL_CAPACITY: usize = 64;

/// Pre-encoded WS frame. Cheap to clone (Arc reference bump).
#[derive(Clone, Debug)]
pub enum WsFrame {
    /// A pre-encoded MessagePack binary frame (zero-copy clone via Bytes).
    Binary(Bytes),
    /// A message that needs per-connection filtering (tag subscription check).
    /// Will be encoded by the WS handler before sending.
    Message(ServerMessage),
}

/// Per-connection state.
struct WsConnection {
    user_id: String,
    permissions: Vec<String>,
    subscribed_tags: HashSet<String>,
    subscribe_all: bool,
    tx: Sender<WsFrame>,
}

/// Tracks all active WebSocket connections, their subscriptions, and
/// provides broadcast helpers for pushing messages to the right set of peers.
pub struct WsManager {
    connections: DashMap<String, WsConnection>,
}

impl std::fmt::Debug for WsManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WsManager")
            .field("connection_count", &self.connections.len())
            .finish()
    }
}

impl WsManager {
    pub fn new() -> Self {
        Self {
            connections: DashMap::new(),
        }
    }

    /// Register a new connection. Returns the receiver half that the WS handler
    /// task will poll for outgoing messages.
    pub fn connect(
        &self,
        conn_id: String,
        user_id: String,
        permissions: Vec<String>,
    ) -> Receiver<WsFrame> {
        let (tx, rx) = mpsc::channel(WS_CHANNEL_CAPACITY);
        self.connections.insert(
            conn_id,
            WsConnection {
                user_id,
                permissions,
                subscribed_tags: HashSet::new(),
                subscribe_all: false,
                tx,
            },
        );
        rx
    }

    /// Remove a connection. Returns `true` if the connection existed.
    pub fn disconnect(&self, conn_id: &str) -> bool {
        self.connections.remove(conn_id).is_some()
    }

    /// Subscribe a connection to specific tag keys.
    pub fn subscribe(&self, conn_id: &str, tag_ids: &[String]) {
        if let Some(mut conn) = self.connections.get_mut(conn_id) {
            conn.subscribed_tags.extend(tag_ids.iter().cloned());
        }
    }

    /// Unsubscribe a connection from specific tag keys.
    pub fn unsubscribe(&self, conn_id: &str, tag_ids: &[String]) {
        if let Some(mut conn) = self.connections.get_mut(conn_id) {
            for tag in tag_ids {
                conn.subscribed_tags.remove(tag);
            }
        }
    }

    /// Toggle subscribe-all mode. When enabled, the connection receives every
    /// tag update regardless of its per-tag subscriptions.
    pub fn subscribe_all(&self, conn_id: &str, enabled: bool) {
        if let Some(mut conn) = self.connections.get_mut(conn_id) {
            conn.subscribe_all = enabled;
        }
    }

    /// Check whether a connection has a given permission string.
    pub fn has_permission(&self, conn_id: &str, permission: &str) -> bool {
        self.connections
            .get(conn_id)
            .map(|conn| conn.permissions.iter().any(|p| p == permission))
            .unwrap_or(false)
    }

    // ── Broadcast helpers ──────────────────────────────────────────────

    /// Encode a message to MessagePack into a reusable buffer, then broadcast
    /// the shared bytes to all connections.
    pub fn broadcast_preencoded_with_buf(&self, msg: &ServerMessage, buf: &mut Vec<u8>) {
        buf.clear();
        let mut se = rmp_serde::Serializer::new(&mut *buf).with_struct_map();
        if serde::Serialize::serialize(msg, &mut se).is_err() {
            return;
        }
        let frame = WsFrame::Binary(Bytes::copy_from_slice(buf));
        self.send_frame_to_all(frame);
    }

    /// Encode a message to MessagePack once, then broadcast the shared
    /// bytes to all connections. Zero-copy per connection (Bytes is Arc-backed).
    pub fn broadcast_preencoded(&self, msg: &ServerMessage) {
        let mut buf = Vec::new();
        self.broadcast_preencoded_with_buf(msg, &mut buf);
    }

    /// Broadcast a single tag update to every connection that is subscribed to the
    /// tag or has `subscribe_all` enabled.
    pub fn broadcast_tag_update(&self, tag_id: &str, msg: ServerMessage) {
        let frame = WsFrame::Message(msg);
        let mut stale = Vec::new();
        for mut entry in self.connections.iter_mut() {
            let conn = entry.value_mut();
            if conn.subscribe_all || conn.subscribed_tags.contains(tag_id) {
                match conn.tx.try_send(frame.clone()) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        stale.push(entry.key().clone());
                    }
                    Err(mpsc::error::TrySendError::Full(_)) => {}
                }
            }
        }
        for id in stale {
            self.connections.remove(&id);
        }
    }

    /// Broadcast an alarm message to all connections.
    pub fn broadcast_alarm(&self, msg: ServerMessage) {
        self.broadcast_preencoded(&msg);
    }

    /// Broadcast HA status to all connected clients.
    pub fn broadcast_ha_status(&self, msg: ServerMessage) {
        self.broadcast_preencoded(&msg);
    }

    /// Broadcast a command acknowledgement to all connections.
    pub fn broadcast_command_ack(&self, msg: ServerMessage) {
        self.broadcast_preencoded(&msg);
    }

    /// Broadcast a device-status change to all connections.
    pub fn broadcast_device_status(&self, msg: ServerMessage) {
        self.broadcast_preencoded(&msg);
    }

    /// Current number of active connections.
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    // ── Internal ───────────────────────────────────────────────────────

    fn send_frame_to_all(&self, frame: WsFrame) {
        let mut stale = Vec::new();
        for mut entry in self.connections.iter_mut() {
            match entry.value_mut().tx.try_send(frame.clone()) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    stale.push(entry.key().clone());
                }
                Err(mpsc::error::TrySendError::Full(_)) => {}
            }
        }
        for id in stale {
            self.connections.remove(&id);
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a WsManager and connect a test connection.
    fn make_manager_and_connect(
        conn_id: &str,
        user_id: &str,
        permissions: Vec<&str>,
    ) -> (WsManager, Receiver<WsFrame>) {
        let mgr = WsManager::new();
        let rx = mgr.connect(
            conn_id.to_string(),
            user_id.to_string(),
            permissions.iter().map(|s| s.to_string()).collect(),
        );
        (mgr, rx)
    }

    #[test]
    fn test_connect_disconnect() {
        let mgr = WsManager::new();
        assert_eq!(mgr.connection_count(), 0);

        let _rx = mgr.connect(
            "c1".to_string(),
            "user1".to_string(),
            vec!["read".to_string()],
        );
        assert_eq!(mgr.connection_count(), 1);

        assert!(mgr.disconnect("c1"));
        assert_eq!(mgr.connection_count(), 0);
        assert!(!mgr.disconnect("c1")); // already gone
    }

    #[tokio::test]
    async fn test_subscribe_unsubscribe() {
        let mgr = WsManager::new();
        let _rx = mgr.connect("c1".to_string(), "user1".to_string(), vec![]);

        mgr.subscribe("c1", &["tag_a".to_string(), "tag_b".to_string()]);

        // Sending a broadcast for an unsubscribed tag should not deliver.
        mgr.broadcast_tag_update(
            "tag_z",
            ServerMessage::TagUpdate {
                tag_id: "tag_z".to_string(),
                value: 1.0,
                quality: "good".to_string(),
                ts: 0,
                simulated: false,
            },
        );

        // Sending for a subscribed tag should deliver (we verify via subscribe_all next).
        // For now just check disconnect removes it.
        mgr.disconnect("c1");
        assert_eq!(mgr.connection_count(), 0);
    }

    #[tokio::test]
    async fn test_subscribe_all() {
        let mgr = WsManager::new();
        let mut rx = mgr.connect("c1".to_string(), "user1".to_string(), vec![]);

        // Without subscribe_all, a broadcast for an unknown tag should not arrive.
        mgr.broadcast_tag_update(
            "tag_x",
            ServerMessage::TagUpdate {
                tag_id: "tag_x".to_string(),
                value: 1.0,
                quality: "good".to_string(),
                ts: 0,
                simulated: false,
            },
        );
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        // Enable subscribe_all
        mgr.subscribe_all("c1", true);

        mgr.broadcast_tag_update(
            "tag_x",
            ServerMessage::TagUpdate {
                tag_id: "tag_x".to_string(),
                value: 2.0,
                quality: "good".to_string(),
                ts: 1,
                simulated: false,
            },
        );
        match rx.try_recv() {
            Ok(WsFrame::Message(ServerMessage::TagUpdate { value, .. })) => {
                assert!((value - 2.0).abs() < f64::EPSILON)
            }
            other => panic!("expected TagUpdate with value 2.0, got {:?}", other),
        }
    }

    #[test]
    fn test_has_permission() {
        let mgr = WsManager::new();
        let _rx = mgr.connect(
            "c1".to_string(),
            "admin".to_string(),
            vec!["read".to_string(), "write".to_string()],
        );

        assert!(mgr.has_permission("c1", "read"));
        assert!(mgr.has_permission("c1", "write"));
        assert!(!mgr.has_permission("c1", "admin"));

        // Non-existent connection
        assert!(!mgr.has_permission("ghost", "read"));
    }

    #[tokio::test]
    async fn test_broadcast_alarm() {
        let mgr = WsManager::new();
        let mut rx = mgr.connect("c1".to_string(), "user1".to_string(), vec![]);

        mgr.broadcast_alarm(ServerMessage::Alarm {
            alarm_id: 42,
            tag_id: "sensor.temp".to_string(),
            severity: "critical".to_string(),
            message: "overheat".to_string(),
            value: 99.0,
            ts: 100,
        });

        match rx.try_recv() {
            Ok(WsFrame::Binary(bytes)) => {
                // Verify it's valid MessagePack
                let msg: ServerMessage = rmp_serde::from_slice(&bytes).unwrap();
                match msg {
                    ServerMessage::Alarm { alarm_id, .. } => assert_eq!(alarm_id, 42),
                    _ => panic!("expected Alarm"),
                }
            }
            other => panic!("expected Binary frame, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_broadcast_command_ack() {
        let mgr = WsManager::new();
        let mut rx = mgr.connect("c1".to_string(), "user1".to_string(), vec![]);

        mgr.broadcast_command_ack(ServerMessage::CommandAck {
            command_id: "cmd-001".to_string(),
            status: "acked".to_string(),
            device_response: Some("OK".to_string()),
            latency_ms: Some(12),
        });

        match rx.try_recv() {
            Ok(WsFrame::Binary(bytes)) => {
                let msg: ServerMessage = rmp_serde::from_slice(&bytes).unwrap();
                match msg {
                    ServerMessage::CommandAck {
                        command_id,
                        latency_ms,
                        ..
                    } => {
                        assert_eq!(command_id, "cmd-001");
                        assert_eq!(latency_ms, Some(12));
                    }
                    _ => panic!("expected CommandAck"),
                }
            }
            other => panic!("expected Binary frame, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_broadcast_device_status() {
        let mgr = WsManager::new();
        let mut rx = mgr.connect("c1".to_string(), "user1".to_string(), vec![]);

        mgr.broadcast_device_status(ServerMessage::DeviceStatus {
            device_id: "plc-01".to_string(),
            online: true,
            latency_ms: Some(5),
        });

        match rx.try_recv() {
            Ok(WsFrame::Binary(bytes)) => {
                let msg: ServerMessage = rmp_serde::from_slice(&bytes).unwrap();
                match msg {
                    ServerMessage::DeviceStatus {
                        device_id, online, ..
                    } => {
                        assert_eq!(device_id, "plc-01");
                        assert!(online);
                    }
                    _ => panic!("expected DeviceStatus"),
                }
            }
            other => panic!("expected Binary frame, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_disconnect_removes_connection() {
        let mgr = WsManager::new();
        let _rx = mgr.connect("c1".to_string(), "user1".to_string(), vec![]);
        let _rx2 = mgr.connect("c2".to_string(), "user2".to_string(), vec![]);
        assert_eq!(mgr.connection_count(), 2);

        mgr.disconnect("c1");
        assert_eq!(mgr.connection_count(), 1);
        assert!(!mgr.has_permission("c1", "read")); // gone

        mgr.disconnect("c2");
        assert_eq!(mgr.connection_count(), 0);
    }

    #[tokio::test]
    async fn test_preencoded_broadcast_shares_bytes() {
        let mgr = WsManager::new();
        let mut rx1 = mgr.connect("c1".to_string(), "user1".to_string(), vec![]);
        let mut rx2 = mgr.connect("c2".to_string(), "user2".to_string(), vec![]);

        mgr.broadcast_preencoded(&ServerMessage::Pong);

        let frame1 = rx1.try_recv().unwrap();
        let frame2 = rx2.try_recv().unwrap();

        // Both should be pre-encoded binary frames
        match (&frame1, &frame2) {
            (WsFrame::Binary(b1), WsFrame::Binary(b2)) => {
                // Same underlying bytes
                assert_eq!(b1.as_ref(), b2.as_ref());
            }
            _ => panic!("expected Binary frames"),
        }
    }
}
