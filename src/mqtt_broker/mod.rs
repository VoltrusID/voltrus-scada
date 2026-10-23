pub mod auth;
pub mod config;

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use tokio::sync::{RwLock, broadcast, watch};

use crate::models::MqttBrokerConfig;

/// Runtime status of the embedded MQTT broker.
#[derive(Debug, Clone, serde::Serialize, PartialEq, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum BrokerStatus {
    Stopped,
    Starting,
    Running,
    Error(String),
}

/// Connected client info.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ConnectedClient {
    pub client_id: String,
    pub connected_at: i64,
    pub subscriptions: Vec<String>,
}

/// Tracked information for a single MQTT topic.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct TopicInfo {
    pub topic: String,
    #[serde(with = "bytes_to_string")]
    pub last_payload: Vec<u8>,
    pub last_update: i64,
    pub message_count: u64,
    pub retain: bool,
}

mod bytes_to_string {
    use serde::Serializer;
    pub fn serialize<S: Serializer>(data: &Vec<u8>, s: S) -> Result<S::Ok, S::Error> {
        match String::from_utf8(data.clone()) {
            Ok(st) => s.serialize_str(&st),
            Err(_) => s.serialize_str(&hex::encode(data)),
        }
    }
}

/// A single recent message for the message stream.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct RecentMessage {
    pub topic: String,
    #[serde(with = "bytes_to_string")]
    pub payload: Vec<u8>,
    pub timestamp: i64,
    pub retain: bool,
}

/// A single MQTT message broadcast to event consumers (e.g. the flow engine,
/// which turns broker messages into flow triggers).
#[derive(Debug, Clone, serde::Serialize)]
pub struct MqttEvent {
    pub topic: String,
    #[serde(with = "bytes_to_string")]
    pub payload: Vec<u8>,
    pub timestamp: i64,
    pub retain: bool,
}

const MAX_RECENT_MESSAGES: usize = 500;
const MAX_TOPIC_MESSAGES: usize = 50;
const EVENT_CHANNEL_CAPACITY: usize = 1024;

/// In-memory tracker of topics and messages passing through the broker.
#[derive(Clone)]
pub struct TopicTracker {
    topics: Arc<RwLock<HashMap<String, TopicInfo>>>,
    recent_messages: Arc<RwLock<VecDeque<RecentMessage>>>,
    event_tx: broadcast::Sender<MqttEvent>,
}

impl TopicTracker {
    pub fn new() -> Self {
        Self {
            topics: Arc::new(RwLock::new(HashMap::new())),
            recent_messages: Arc::new(RwLock::new(VecDeque::with_capacity(MAX_RECENT_MESSAGES))),
            event_tx: broadcast::channel(EVENT_CHANNEL_CAPACITY).0,
        }
    }

    /// Subscribe to a broadcast of every message that flows through the broker.
    /// Used by the flow engine as the MQTT event source.
    pub fn subscribe_events(&self) -> broadcast::Receiver<MqttEvent> {
        self.event_tx.subscribe()
    }

    pub async fn record_message(&self, topic: String, payload: Vec<u8>, retain: bool) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        // Fan out to event subscribers (best-effort; no receivers = no-op).
        let _ = self.event_tx.send(MqttEvent {
            topic: topic.clone(),
            payload: payload.clone(),
            timestamp: now,
            retain,
        });

        {
            let mut topics = self.topics.write().await;
            let entry = topics.entry(topic.clone()).or_insert_with(|| TopicInfo {
                topic: topic.clone(),
                last_payload: Vec::new(),
                last_update: 0,
                message_count: 0,
                retain: false,
            });
            entry.last_payload = payload.clone();
            entry.last_update = now;
            entry.message_count += 1;
            entry.retain = retain;
        }

        {
            let mut msgs = self.recent_messages.write().await;
            if msgs.len() >= MAX_RECENT_MESSAGES {
                msgs.pop_front();
            }
            msgs.push_back(RecentMessage {
                topic,
                payload,
                timestamp: now,
                retain,
            });
        }
    }

    pub async fn get_topics(&self) -> Vec<TopicInfo> {
        let topics = self.topics.read().await;
        let mut list: Vec<TopicInfo> = topics.values().cloned().collect();
        list.sort_by(|a, b| a.topic.cmp(&b.topic));
        list
    }

    pub async fn get_topic_messages(&self, topic_prefix: &str) -> Vec<RecentMessage> {
        let msgs = self.recent_messages.read().await;
        let prefix_slash = format!("{}/", topic_prefix);
        let mut filtered: Vec<RecentMessage> = msgs
            .iter()
            .filter(|m| m.topic == topic_prefix || m.topic.starts_with(&prefix_slash))
            .rev()
            .take(MAX_TOPIC_MESSAGES)
            .cloned()
            .collect();
        filtered.reverse();
        filtered
    }

    pub async fn get_recent_messages(&self, limit: usize) -> Vec<RecentMessage> {
        let msgs = self.recent_messages.read().await;
        let mut result: Vec<RecentMessage> = msgs.iter().rev().take(limit).cloned().collect();
        result.reverse();
        result
    }

    pub async fn clear(&self) {
        self.topics.write().await.clear();
        self.recent_messages.write().await.clear();
    }
}

impl Default for TopicTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a hierarchical tree from a flat list of topic paths.
fn build_topic_tree(topics: &[TopicInfo]) -> serde_json::Value {
    use serde_json::{Map, Value};

    let mut root = Map::new();

    for info in topics {
        let parts: Vec<&str> = info.topic.split('/').collect();
        let node = &mut root;

        for (i, part) in parts.iter().enumerate() {
            let is_leaf = i == parts.len() - 1;
            let children = node
                .entry(*part)
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .unwrap();

            if is_leaf {
                children.insert("_leaf".to_string(), Value::Bool(true));
                children.insert(
                    "_message_count".to_string(),
                    Value::Number(info.message_count.into()),
                );
                children.insert(
                    "_last_update".to_string(),
                    Value::Number(info.last_update.into()),
                );
            }
        }
    }

    Value::Object(root)
}

/// Shared broker state accessible from API handlers.
#[derive(Clone)]
pub struct MqttBrokerState {
    pub status: Arc<watch::Sender<BrokerStatus>>,
    pub status_rx: watch::Receiver<BrokerStatus>,
    pub config: Arc<RwLock<Option<MqttBrokerConfig>>>,
    pub clients: Arc<RwLock<Vec<ConnectedClient>>>,
    pub shutdown_tx: Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    pub topic_tracker: TopicTracker,
    publish_tx: Arc<std::sync::Mutex<Option<rumqttd::local::LinkTx>>>,
}

impl MqttBrokerState {
    pub fn new() -> Self {
        let (status_tx, status_rx) = watch::channel(BrokerStatus::Stopped);
        Self {
            status: Arc::new(status_tx),
            status_rx,
            config: Arc::new(RwLock::new(None)),
            clients: Arc::new(RwLock::new(Vec::new())),
            shutdown_tx: Arc::new(tokio::sync::Mutex::new(None)),
            topic_tracker: TopicTracker::new(),
            publish_tx: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    pub async fn current_status(&self) -> BrokerStatus {
        self.status_rx.borrow().clone()
    }

    pub fn publish_sync(&self, topic: &str, payload: &[u8]) -> Result<(), String> {
        let mut guard = self.publish_tx.lock().map_err(|e| e.to_string())?;
        if let Some(ref mut link_tx) = *guard {
            link_tx
                .publish(topic.to_string(), payload.to_vec())
                .map_err(|e| format!("Publish failed: {}", e))?;
            Ok(())
        } else {
            Err("Broker is not running".to_string())
        }
    }
}

impl Default for MqttBrokerState {
    fn default() -> Self {
        Self::new()
    }
}

/// Start the embedded MQTT broker as a background task.
pub async fn start_broker(state: MqttBrokerState, config: MqttBrokerConfig) -> Result<(), String> {
    if state.current_status().await == BrokerStatus::Running {
        return Err("Broker is already running".to_string());
    }

    let _ = state.status.send(BrokerStatus::Starting);

    {
        let mut cfg = state.config.write().await;
        *cfg = Some(config.clone());
    }

    let broker_config = config::build_rumqttd_config(&config);
    let port = config.port;
    let status_tx = state.status.clone();
    let tracker = state.topic_tracker.clone();
    let publish_tx = state.publish_tx.clone();

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    {
        let mut handle = state.shutdown_tx.lock().await;
        *handle = Some(shutdown_tx);
    }

    let status_tx_clone = status_tx.clone();
    let status_tx_final = status_tx.clone();
    tokio::task::spawn_blocking(move || {
        let mut broker = rumqttd::Broker::new(broker_config);

        // Create local link to intercept all messages
        let (mut link_tx, mut link_rx) = broker
            .link("voltrus-tracker")
            .expect("failed to create local link");

        link_tx
            .subscribe("#")
            .expect("failed to subscribe to wildcard");

        // Store the link_tx for publish capability
        {
            let mut px = publish_tx.lock().unwrap();
            *px = Some(link_tx);
        }

        let tracker_clone = tracker.clone();

        // Spawn notification processor thread
        let _notif_handle = std::thread::Builder::new()
            .name("mqtt-tracker".into())
            .spawn(move || {
                let rt = tokio::runtime::Handle::current();
                loop {
                    match link_rx.recv() {
                        Ok(Some(rumqttd::Notification::Forward(forward))) => {
                            let topic = String::from_utf8_lossy(&forward.publish.topic).to_string();
                            let payload = forward.publish.payload.to_vec();
                            let retain = forward.publish.retain;

                            let t = tracker_clone.clone();
                            rt.block_on(t.record_message(topic, payload, retain));
                        }
                        Ok(None) | Ok(Some(rumqttd::Notification::Unschedule)) => {}
                        Ok(Some(_)) => {}
                        Err(_) => break,
                    }
                }
            });

        let broker_handle = std::thread::spawn(move || {
            if let Err(e) = broker.start() {
                tracing::error!("MQTT broker error: {}", e);
            }
        });

        let _ = shutdown_rx.blocking_recv();

        tracing::info!("MQTT broker shutting down on port {}", port);
        let _ = status_tx_clone.send(BrokerStatus::Stopped);

        {
            let mut px = publish_tx.lock().unwrap();
            *px = None;
        }

        drop(broker_handle);
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let _ = status_tx_final.send(BrokerStatus::Running);
    tracing::info!("MQTT broker started on port {}", port);

    Ok(())
}

/// Stop the embedded MQTT broker.
pub async fn stop_broker(state: MqttBrokerState) -> Result<(), String> {
    if state.current_status().await != BrokerStatus::Running {
        return Err("Broker is not running".to_string());
    }

    let mut handle = state.shutdown_tx.lock().await;
    if let Some(tx) = handle.take() {
        let _ = tx.send(());
    }

    {
        let mut clients = state.clients.write().await;
        clients.clear();
    }

    state.topic_tracker.clear().await;

    let _ = state.status.send(BrokerStatus::Stopped);
    tracing::info!("MQTT broker stopped");

    Ok(())
}

/// Get the topic tree for the API layer.
pub async fn get_topic_tree(tracker: &TopicTracker) -> serde_json::Value {
    let topics = tracker.get_topics().await;
    let tree = build_topic_tree(&topics);
    serde_json::json!({
        "topics": topics,
        "tree": tree,
    })
}

#[cfg(test)]
mod topic_tracker_tests {
    use super::*;

    #[tokio::test]
    async fn record_message_broadcasts_to_subscribers() {
        let tracker = TopicTracker::new();
        let mut rx = tracker.subscribe_events();

        tracker
            .record_message("sensors/01/temp".into(), b"24.5".to_vec(), false)
            .await;

        let ev = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv())
            .await
            .expect("event broadcast timed out")
            .expect("event channel closed");
        assert_eq!(ev.topic, "sensors/01/temp");
        assert_eq!(ev.payload, b"24.5");
        assert!(!ev.retain);
    }

    #[tokio::test]
    async fn record_message_without_subscribers_is_noop() {
        // No subscriber present — the broadcast send must not error.
        let tracker = TopicTracker::new();
        tracker
            .record_message("x/y".into(), b"1".to_vec(), true)
            .await;
    }
}
