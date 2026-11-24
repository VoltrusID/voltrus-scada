use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::cache::DeviceCache;
#[cfg(feature = "modbus")]
use crate::modbus::ModbusPool;
use crate::storage::Storage;
use crate::ws_manager::WsManager;

pub const COMMAND_EXPIRY_SECS: i64 = 1800; // 30 minutes
pub const POLL_INTERVAL_MS: u64 = 500;
/// Longest a single write may take, whatever the protocol, before it counts
/// as failed and the device is released.
pub const WRITE_TIMEOUT_SECS: u64 = 30;
/// Attempts at recording that the device acknowledged a write.
pub const ACK_RECORD_ATTEMPTS: u32 = 4;

#[derive(Clone)]
pub struct CommandQueue {
    pub(super) storage: Storage,
    pub(super) cache: DeviceCache,
    pub(super) in_flight: Arc<RwLock<HashSet<i64>>>,
    pub(super) ws_manager: Option<Arc<WsManager>>,
    /// Acknowledged writes waiting for a poll to read them back, by command id.
    pub(super) readback_trackers:
        Arc<RwLock<std::collections::HashMap<String, super::readback::PendingReadback>>>,
    #[cfg(feature = "modbus")]
    pub(super) modbus_pool: Arc<ModbusPool>,
    /// Embedded MQTT broker handle for MQTT command writes (publish). Shared
    /// via an `Arc<RwLock<Option<…>>>` so main.rs can populate it after the
    /// broker starts, even though the queue task is already running.
    #[cfg(feature = "mqtt-broker")]
    pub(super) mqtt_broker: Arc<RwLock<Option<crate::mqtt_broker::MqttBrokerState>>>,
}
