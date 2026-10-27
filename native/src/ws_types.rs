//! WebSocket wire types — mirror of the voltrus server's `src/ws_types.rs`.
//!
//! Wire format: binary MessagePack frames (rmp-serde), serde tag = "type".
//! Keep this file in sync with the server; `tests/ws_roundtrip.rs` guards
//! the encoding on both sides.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagUpdateEntry {
    pub tag_id: String,
    pub value: f64,
    pub quality: String,
    pub ts: i64,
    pub simulated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    Auth {
        token: String,
    },
    Subscribe {
        tag_ids: Vec<String>,
    },
    Unsubscribe {
        tag_ids: Vec<String>,
    },
    TagWrite {
        tag_id: String,
        value: f64,
        command_id: Option<String>,
    },
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    AuthOk {
        user: String,
        permissions: Vec<String>,
    },
    AuthError {
        message: String,
    },
    TagUpdate {
        tag_id: String,
        value: f64,
        quality: String,
        ts: i64,
        simulated: bool,
    },
    /// Batch of tag updates sent in a single message to reduce per-message overhead.
    TagUpdateBatch {
        updates: Vec<TagUpdateEntry>,
    },
    CommandAck {
        command_id: String,
        status: String,
        device_response: Option<String>,
        latency_ms: Option<u64>,
    },
    DeviceStatus {
        device_id: String,
        online: bool,
        latency_ms: Option<u64>,
    },
    Alarm {
        alarm_id: i64,
        tag_id: String,
        severity: String,
        message: String,
        value: f64,
        ts: i64,
    },
    /// An existing alarm changed state: `event` is `acknowledged`, `resolved`,
    /// `shelved` or `unshelved`; `actor` is `None` when the system did it.
    AlarmUpdate {
        alarm_id: i64,
        tag_id: String,
        event: String,
        state: String,
        actor: Option<String>,
        ts: i64,
    },
    Pong,
    HaStatus {
        role: String,
        peer_status: String,
        sync_lag_ms: Option<u64>,
        node_id: String,
        peer_id: Option<String>,
        failover_ts: Option<String>,
    },
}
