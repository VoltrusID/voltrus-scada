use crate::cache::DeviceCache;
use crate::flow_engine::{
    message::Message, node::NodeConfig, run_log, state::FlowState, trigger::TriggerConfig,
};
use crate::storage::Storage;
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};

#[cfg(feature = "mqtt-broker")]
use crate::mqtt_broker::{MqttEvent, TopicTracker};
#[cfg(feature = "mqtt-broker")]
use std::sync::Mutex;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Flow {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub trigger: TriggerConfig,
    pub nodes: Vec<NodeConfig>,
}

pub struct FlowEngine {
    flows: Arc<RwLock<HashMap<String, Flow>>>,
    state: Arc<FlowState>,
    storage: Storage,
    cache: DeviceCache,
    in_flight: Arc<DashMap<String, ()>>,
    last_fired: Arc<DashMap<String, std::time::Instant>>,
    last_values: Arc<DashMap<String, serde_json::Value>>,
    #[cfg(feature = "mqtt-broker")]
    broker_tracker: Arc<Mutex<Option<TopicTracker>>>,
}

impl FlowEngine {
    pub fn new(storage: Storage, cache: DeviceCache) -> Self {
        Self {
            flows: Arc::new(RwLock::new(HashMap::new())),
            state: Arc::new(FlowState::new()),
            storage,
            cache,
            in_flight: Arc::new(DashMap::new()),
            last_fired: Arc::new(DashMap::new()),
            last_values: Arc::new(DashMap::new()),
            #[cfg(feature = "mqtt-broker")]
            broker_tracker: Arc::new(Mutex::new(None)),
        }
    }

    /// Attach the embedded MQTT broker's message stream so flows with MQTT
    /// triggers fire on broker messages. Optional and only meaningful with the
    /// `mqtt-broker` feature; the engine runs fine without it.
    #[cfg(feature = "mqtt-broker")]
    pub fn set_broker_tracker(&self, tracker: TopicTracker) {
        if let Ok(mut guard) = self.broker_tracker.lock() {
            *guard = Some(tracker);
        }
    }

    pub async fn spawn(&self) {
        let flows = self.flows.clone();
        let state = self.state.clone();
        let storage = self.storage.clone();
        let cache = self.cache.clone();
        let mut rx = self.cache.subscribe();
        let in_flight = self.in_flight.clone();
        let last_fired = self.last_fired.clone();
        let last_values = self.last_values.clone();

        // Spawn timer flows task
        {
            let flows = flows.clone();
            let state = state.clone();
            let storage = storage.clone();
            let cache = cache.clone();
            let in_flight = in_flight.clone();
            let last_fired = last_fired.clone();
            let last_values = last_values.clone();
            tokio::spawn(async move {
                let mut last_run: HashMap<String, std::time::Instant> = HashMap::new();
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    let flows_guard = flows.read().await;
                    for flow in flows_guard.values() {
                        if !flow.enabled {
                            continue;
                        }
                        if let TriggerConfig::Timer { interval_ms } = &flow.trigger {
                            let now = std::time::Instant::now();
                            let should_run = match last_run.get(&flow.id) {
                                Some(last) => {
                                    now.duration_since(*last).as_millis() as u64 >= *interval_ms
                                }
                                None => true,
                            };
                            if should_run {
                                last_run.insert(flow.id.clone(), now);
                                let msg = Message::new(
                                    serde_json::json!({"timestamp": crate::timeutil::now_secs()}),
                                );
                                if gate_fire(flow, &msg, &in_flight, &last_fired, &last_values) {
                                    let flow = flow.clone();
                                    let state = state.clone();
                                    let storage = storage.clone();
                                    let cache = cache.clone();
                                    let in_flight = in_flight.clone();
                                    let flow_id = flow.id.clone();
                                    tokio::spawn(async move {
                                        run_flow(&flow, msg, state.as_ref(), &storage, &cache)
                                            .await;
                                        in_flight.remove(&flow_id);
                                    });
                                }
                            }
                        }
                    }
                }
            });
        }

        // MQTT event source: feed broker messages into the engine as Messages.
        // Only compiled when the embedded broker is enabled; without it the
        // MQTT trigger variant still deserialises but never fires.
        #[cfg(feature = "mqtt-broker")]
        {
            let broker_tracker = self.broker_tracker.clone();
            let flows = flows.clone();
            let state = state.clone();
            let storage = storage.clone();
            let cache = cache.clone();
            let in_flight = in_flight.clone();
            let last_fired = last_fired.clone();
            let last_values = last_values.clone();
            tokio::spawn(async move {
                loop {
                    // Acquire a receiver without holding the non-Send
                    // `MutexGuard` across the await below.
                    let rx = {
                        let Ok(guard) = broker_tracker.lock() else {
                            break;
                        };
                        (*guard).as_ref().map(|tracker| tracker.subscribe_events())
                    };
                    let mut rx = match rx {
                        Some(rx) => rx,
                        None => {
                            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                            continue;
                        }
                    };
                    loop {
                        match rx.recv().await {
                            Ok(event) => {
                                let msg = mqtt_event_to_message(event);
                                let flows_guard = flows.read().await;
                                for flow in flows_guard.values() {
                                    if !flow.enabled {
                                        continue;
                                    }
                                    if flow.trigger.matches(&msg)
                                        && gate_fire(
                                            flow,
                                            &msg,
                                            &in_flight,
                                            &last_fired,
                                            &last_values,
                                        )
                                    {
                                        let flow = flow.clone();
                                        let state = state.clone();
                                        let storage = storage.clone();
                                        let cache = cache.clone();
                                        let msg = msg.clone();
                                        let in_flight = in_flight.clone();
                                        let flow_id = flow.id.clone();
                                        tokio::spawn(async move {
                                            run_flow(&flow, msg, state.as_ref(), &storage, &cache)
                                                .await;
                                            in_flight.remove(&flow_id);
                                        });
                                    }
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                tracing::warn!("FlowEngine MQTT source lagged by {} messages", n);
                            }
                            Err(broadcast::error::RecvError::Closed) => {
                                tracing::debug!(
                                    "FlowEngine MQTT event stream closed; resubscribing"
                                );
                                break;
                            }
                        }
                    }
                }
            });
        }

        // Main loop: process broadcast messages
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(snapshot) => {
                        let msg = device_snapshot_to_message(snapshot);
                        let flows_guard = flows.read().await;
                        for flow in flows_guard.values() {
                            if !flow.enabled {
                                continue;
                            }
                            if flow.trigger.matches(&msg)
                                && gate_fire(flow, &msg, &in_flight, &last_fired, &last_values)
                            {
                                let flow = flow.clone();
                                let state = state.clone();
                                let storage = storage.clone();
                                let cache = cache.clone();
                                let msg = msg.clone();
                                let in_flight = in_flight.clone();
                                let flow_id = flow.id.clone();
                                tokio::spawn(async move {
                                    run_flow(&flow, msg, state.as_ref(), &storage, &cache).await;
                                    in_flight.remove(&flow_id);
                                });
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("FlowEngine lagged by {} messages", n);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        tracing::info!("FlowEngine broadcast channel closed");
                        break;
                    }
                }
            }
        });
    }

    pub async fn add_flow(&self, flow: Flow) {
        let mut flows = self.flows.write().await;
        flows.insert(flow.id.clone(), flow);
    }

    pub async fn remove_flow(&self, flow_id: &str) {
        let mut flows = self.flows.write().await;
        flows.remove(flow_id);
    }

    pub async fn get_flow(&self, flow_id: &str) -> Option<Flow> {
        let flows = self.flows.read().await;
        flows.get(flow_id).cloned()
    }

    pub async fn list_flows(&self) -> Vec<Flow> {
        let flows = self.flows.read().await;
        flows.values().cloned().collect()
    }

    pub async fn update_flow(&self, flow: Flow) {
        let mut flows = self.flows.write().await;
        flows.insert(flow.id.clone(), flow);
    }
}

fn device_snapshot_to_message(snapshot: crate::models::DeviceSnapshot) -> Message {
    let mut metadata = HashMap::new();
    // Use the snapshot's real acquisition protocol when known so triggers can
    // discriminate by source; fall back to "modbus" for back-compat with
    // snapshots that predate protocol tagging.
    let source = snapshot
        .protocol
        .clone()
        .unwrap_or_else(|| "modbus".to_string());
    metadata.insert("source".to_string(), source);
    let payload = serde_json::json!({
        "device_id": snapshot.device_id,
        "status": match snapshot.status {
            crate::models::DeviceStatus::Online => "online",
            crate::models::DeviceStatus::Offline => "offline",
        },
        "timestamp": snapshot.timestamp,
        "metrics": snapshot.metrics,
        "simulated": snapshot.simulated,
    });
    let mut msg = Message::new(payload);
    msg.metadata = metadata;
    msg
}

/// Build a flow-engine [`Message`] from a broker MQTT event. Payload bytes are
/// parsed as UTF-8 then JSON when possible; otherwise wrapped as `{"value": ..}`
/// (raw string, or hex if the bytes are not valid UTF-8). `source` and `topic`
/// land in metadata so the MQTT trigger can match on them.
#[cfg(feature = "mqtt-broker")]
fn mqtt_event_to_message(event: MqttEvent) -> Message {
    let mut metadata = HashMap::new();
    metadata.insert("source".to_string(), "mqtt".to_string());
    metadata.insert("topic".to_string(), event.topic.clone());

    let payload = match String::from_utf8(event.payload.clone()) {
        Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(value) => value,
            Err(_) => serde_json::json!({ "value": text }),
        },
        Err(_) => serde_json::json!({ "value": hex::encode(&event.payload) }),
    };

    let mut msg = Message::new(payload);
    msg.metadata = metadata;
    msg
}

async fn run_flow(
    flow: &Flow,
    mut msg: Message,
    state: &FlowState,
    storage: &Storage,
    cache: &DeviceCache,
) {
    let start = std::time::Instant::now();
    // Execute process nodes (in array order) before output nodes so the grouped
    // canvas layout matches the real execution order.
    let process_nodes: Vec<&NodeConfig> =
        flow.nodes.iter().filter(|n| !is_output_node(n)).collect();
    let output_nodes: Vec<&NodeConfig> = flow.nodes.iter().filter(|n| is_output_node(n)).collect();
    for node in process_nodes.into_iter().chain(output_nodes) {
        let ctx = crate::flow_engine::node::NodeContext {
            flow_id: flow.id.clone(),
            node_id: node_id(node),
            state: state.clone(),
            storage: storage.clone(),
            cache: cache.clone(),
        };
        match node.process(msg, &ctx).await {
            Ok(crate::flow_engine::node::NodeResult::Continue(new_msg)) => {
                msg = new_msg;
            }
            Ok(crate::flow_engine::node::NodeResult::FilterReject) => {
                run_log::log_run(
                    storage,
                    &flow.id,
                    start.elapsed().as_millis() as u64,
                    true,
                    None,
                    None,
                )
                .await;
                return;
            }
            Err(e) => {
                run_log::log_run(
                    storage,
                    &flow.id,
                    start.elapsed().as_millis() as u64,
                    false,
                    Some(e),
                    None,
                )
                .await;
                return;
            }
        }
    }
    run_log::log_run(
        storage,
        &flow.id,
        start.elapsed().as_millis() as u64,
        true,
        None,
        Some(msg.payload),
    )
    .await;
}

fn is_output_node(node: &NodeConfig) -> bool {
    matches!(
        node,
        NodeConfig::DataSourceOut { .. }
            | NodeConfig::HttpOut { .. }
            | NodeConfig::AlarmOut { .. }
            | NodeConfig::WriteOut { .. }
    )
}

/// Re-entrancy / debounce / on-change-only gate shared by every trigger source.
/// Returns true (and records the fire in the trackers) only when the flow is
/// allowed to run now. The caller must clear `in_flight` when the run finishes.
fn gate_fire(
    flow: &Flow,
    msg: &Message,
    in_flight: &Arc<DashMap<String, ()>>,
    last_fired: &Arc<DashMap<String, std::time::Instant>>,
    last_values: &Arc<DashMap<String, serde_json::Value>>,
) -> bool {
    // Re-entrancy: never overlap two runs of the same flow.
    if in_flight.contains_key(&flow.id) {
        return false;
    }
    let now = std::time::Instant::now();
    // Debounce: minimum interval between firings (modbus + mqtt triggers).
    let debounce_ms = match &flow.trigger {
        TriggerConfig::Modbus { debounce_ms, .. } | TriggerConfig::Mqtt { debounce_ms, .. } => {
            *debounce_ms
        }
        TriggerConfig::Timer { .. } => None,
    };
    if let Some(ms) = debounce_ms
        && let Some(last) = last_fired.get(&flow.id)
        && (now.duration_since(*last).as_millis() as u64) < ms
    {
        return false;
    }
    // on_change_only (modbus): suppress unless the watched value actually changed.
    if let TriggerConfig::Modbus {
        metric,
        on_change_only,
        ..
    } = &flow.trigger
        && on_change_only.unwrap_or(false)
    {
        let current = metric
            .as_deref()
            .filter(|s| !s.is_empty())
            .and_then(|k| msg.payload.get("metrics").and_then(|m| m.get(k)))
            .cloned()
            .unwrap_or_else(|| {
                msg.payload
                    .get("metrics")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null)
            });
        let changed = {
            let prev = last_values.get(&flow.id);
            !prev.as_deref().is_some_and(|p| *p == current)
        };
        if !changed {
            return false;
        }
        last_values.insert(flow.id.clone(), current);
    }
    in_flight.insert(flow.id.clone(), ());
    last_fired.insert(flow.id.clone(), now);
    true
}

fn node_id(node: &NodeConfig) -> String {
    match node {
        NodeConfig::Filter { id, .. } => id.clone(),
        NodeConfig::Transform { id, .. } => id.clone(),
        NodeConfig::Threshold { id, .. } => id.clone(),
        NodeConfig::Lookup { id, .. } => id.clone(),
        NodeConfig::Expression { id, .. } => id.clone(),
        NodeConfig::DataSourceOut { id, .. } => id.clone(),
        NodeConfig::HttpOut { id, .. } => id.clone(),
        NodeConfig::AlarmOut { id, .. } => id.clone(),
        NodeConfig::WriteOut { id, .. } => id.clone(),
        NodeConfig::Script { id, .. } => id.clone(),
        NodeConfig::Dsl { id, .. } => id.clone(),
    }
}

#[cfg(all(test, feature = "mqtt-broker"))]
mod mqtt_event_to_message_tests {
    use super::mqtt_event_to_message;
    use crate::mqtt_broker::MqttEvent;

    fn ev(topic: &str, payload: Vec<u8>) -> MqttEvent {
        MqttEvent {
            topic: topic.into(),
            payload,
            timestamp: 0,
            retain: false,
        }
    }

    #[test]
    fn parses_json_payload() {
        let msg = mqtt_event_to_message(ev("t", br#"{"temp":24.5}"#.to_vec()));
        assert_eq!(msg.metadata.get("source"), Some(&"mqtt".to_string()));
        assert_eq!(msg.metadata.get("topic"), Some(&"t".to_string()));
        assert_eq!(msg.payload["temp"], 24.5);
    }

    #[test]
    fn wraps_plain_string() {
        let msg = mqtt_event_to_message(ev("t", b"hello".to_vec()));
        assert_eq!(msg.payload["value"], "hello");
    }

    #[test]
    fn wraps_non_utf8_as_hex() {
        let msg = mqtt_event_to_message(ev("t", vec![0xff, 0x00, 0x01]));
        assert_eq!(msg.payload["value"], "ff0001");
    }
}

#[cfg(test)]
mod gate_fire_tests {
    use super::*;

    fn flow(id: &str, trigger: TriggerConfig) -> Flow {
        Flow {
            id: id.to_string(),
            name: id.to_string(),
            enabled: true,
            trigger,
            nodes: vec![],
        }
    }

    fn trackers() -> (
        Arc<DashMap<String, ()>>,
        Arc<DashMap<String, std::time::Instant>>,
        Arc<DashMap<String, serde_json::Value>>,
    ) {
        (
            Arc::new(DashMap::new()),
            Arc::new(DashMap::new()),
            Arc::new(DashMap::new()),
        )
    }

    #[test]
    fn reentrancy_blocks_overlap() {
        let (in_flight, last_fired, last_values) = trackers();
        let f = flow("f1", TriggerConfig::Timer { interval_ms: 1000 });
        let msg = Message::new(serde_json::json!({}));
        assert!(gate_fire(&f, &msg, &in_flight, &last_fired, &last_values));
        // A second fire while the first run is still in flight must be blocked.
        assert!(!gate_fire(&f, &msg, &in_flight, &last_fired, &last_values));
    }

    #[test]
    fn debounce_suppresses_within_window() {
        let (in_flight, last_fired, last_values) = trackers();
        let f = flow(
            "f1",
            TriggerConfig::Modbus {
                device_id: "*".into(),
                metric: None,
                debounce_ms: Some(10_000),
                on_change_only: Some(false),
            },
        );
        let msg = Message::new(serde_json::json!({"metrics": {"temp": 5}}));
        assert!(gate_fire(&f, &msg, &in_flight, &last_fired, &last_values));
        in_flight.remove("f1"); // previous run finished
        assert!(!gate_fire(&f, &msg, &in_flight, &last_fired, &last_values));
    }

    #[test]
    fn on_change_only_suppresses_unchanged() {
        let (in_flight, last_fired, last_values) = trackers();
        let f = flow(
            "f1",
            TriggerConfig::Modbus {
                device_id: "*".into(),
                metric: Some("temp".into()),
                debounce_ms: None,
                on_change_only: Some(true),
            },
        );
        let m = |v: i64| Message::new(serde_json::json!({"metrics": {"temp": v}}));
        assert!(gate_fire(&f, &m(5), &in_flight, &last_fired, &last_values));
        in_flight.remove("f1");
        assert!(!gate_fire(&f, &m(5), &in_flight, &last_fired, &last_values));
        assert!(gate_fire(&f, &m(6), &in_flight, &last_fired, &last_values));
    }

    #[test]
    fn output_nodes_classified() {
        assert!(is_output_node(&NodeConfig::HttpOut {
            id: "h".into(),
            url: "u".into(),
            headers: Default::default(),
            timeout_ms: 1,
            retry_count: 0
        }));
        assert!(is_output_node(&NodeConfig::WriteOut {
            id: "w".into(),
            data_source_id: 1,
            value_expr: None,
            policy: "immediate".into(),
            reason: "r".into()
        }));
        assert!(!is_output_node(&NodeConfig::Filter {
            id: "f".into(),
            condition: "true".into()
        }));
    }
}
