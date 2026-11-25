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

/// An electronic signature (21 CFR Part 11) sent with an action an
/// Enterprise add-on requires one for: the signed-in user's password, the
/// meaning exactly as the server stated it, and the user's reason. The
/// password never shows in `Debug`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    pub password: String,
    pub meaning: String,
    pub reason: String,
}

impl std::fmt::Debug for Signature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signature")
            .field("meaning", &self.meaning)
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}

/// What the server wants signed: the action (`tag_write`, `alarm_ack`, …)
/// and the meaning to show the user, read-only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SignatureRequired {
    pub action: String,
    pub meaning: String,
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
        /// Only when the server asked for one (Enterprise add-ons).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<Signature>,
    },
    Ping,
    /// Join the live editing session of a screen (docs/collaboration.md).
    DesignJoin {
        screen: String,
        protocol: u32,
        #[serde(default)]
        epoch: Option<String>,
    },
    /// One y-sync v1 message: step 1, step 2 or update.
    DesignSync {
        screen: String,
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    },
    /// A y-protocols awareness update: this console's presence.
    DesignAwareness {
        screen: String,
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    },
    DesignLeave {
        screen: String,
    },
    /// Write the session's pending edits to the draft now (⌘S).
    DesignFlush {
        screen: String,
    },
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
    /// `status` is `signature_required` when the write needs an electronic
    /// signature, or `rejected` with `signature_required` set when the one
    /// sent was refused (a wrong password).
    CommandAck {
        command_id: String,
        status: String,
        device_response: Option<String>,
        latency_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature_required: Option<SignatureRequired>,
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
    DesignJoined {
        screen: String,
        epoch: String,
        resync: bool,
        user: String,
        color: String,
        protocol: u32,
        limits: DesignLimits,
    },
    DesignSync {
        screen: String,
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    },
    DesignAwareness {
        screen: String,
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    },
    DesignError {
        screen: String,
        code: String,
        message: String,
        reload: bool,
    },
    DesignRejected {
        screen: String,
        objects: Vec<DesignRejectedObject>,
    },
    DesignFlushed {
        screen: String,
        seq: i64,
        at_ms: i64,
        users: Vec<String>,
    },
    DesignSessions {
        sessions: Vec<DesignSessionInfo>,
    },
    DesignComment {
        screen: String,
        event: String,
        comment: serde_json::Value,
    },
}

/// The limits a collaboration session enforces, told when joining.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignLimits {
    pub max_update: usize,
    pub max_rate: u32,
    pub max_doc: usize,
    pub max_awareness: usize,
    pub flush_ms: u64,
}

/// An object a flush put back, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignRejectedObject {
    pub object_type: String,
    pub object_id: String,
    pub reason: String,
}

/// The people editing one screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignSessionInfo {
    pub screen: String,
    pub users: Vec<DesignUser>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignUser {
    pub user: String,
    pub color: String,
}
