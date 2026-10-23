use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::cache::DeviceCache;
use crate::health::readback::ReadbackTracker;
#[cfg(feature = "modbus")]
use crate::modbus::ModbusPool;
use crate::storage::Storage;
use crate::ws_manager::WsManager;

pub const COMMAND_EXPIRY_SECS: i64 = 1800; // 30 minutes
pub const POLL_INTERVAL_MS: u64 = 500;

pub struct CommandQueue {
    pub(super) storage: Storage,
    pub(super) cache: DeviceCache,
    pub(super) in_flight: Arc<RwLock<HashSet<i64>>>,
    pub(super) ws_manager: Option<Arc<WsManager>>,
    pub(super) readback_trackers: Arc<RwLock<std::collections::HashMap<String, ReadbackTracker>>>,
    #[cfg(feature = "modbus")]
    pub(super) modbus_pool: Arc<ModbusPool>,
    /// Embedded MQTT broker handle for MQTT command writes (publish). Shared
    /// via an `Arc<RwLock<Option<…>>>` so main.rs can populate it after the
    /// broker starts, even though the queue task is already running.
    #[cfg(feature = "mqtt-broker")]
    pub(super) mqtt_broker: Arc<RwLock<Option<crate::mqtt_broker::MqttBrokerState>>>,
}
