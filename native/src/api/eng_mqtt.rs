//! Data Manager › MQTT broker: the embedded broker's routes
//! (`src/api/mqtt_broker.rs`), mirrored. The status is open to every
//! signed-in user; config, start/stop, topics, messages and publish are
//! operator routes (`demo_protected`).
//!
//! Also the pure parts of the section: the config form's validation and
//! body, the publish body, the topic tree flattened for a keyboard list,
//! and payloads pretty-printed when they are JSON.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::ops::encode;
use super::{Method, VoltrusApi};

pub const STATUS: &str = "/api/v1/mqtt-broker/status";
pub const CONFIG: &str = "/api/v1/mqtt-broker/config";
pub const START: &str = "/api/v1/mqtt-broker/start";
pub const STOP: &str = "/api/v1/mqtt-broker/stop";
pub const TOPICS: &str = "/api/v1/mqtt-broker/topics";
pub const MESSAGES: &str = "/api/v1/mqtt-broker/messages";
pub const PUBLISH: &str = "/api/v1/mqtt-broker/publish";

/// `GET /api/v1/mqtt-broker/topics/{topic}/messages`, the topic encoded
/// as one path segment (as the web client's `encodeURIComponent`).
pub fn topic_messages_path(topic: &str) -> String {
    format!("{TOPICS}/{}/messages", encode(topic))
}

/// `crate::mqtt_broker::ConnectedClient`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct ConnectedClient {
    pub client_id: String,
    /// Unix seconds.
    pub connected_at: i64,
    pub subscriptions: Vec<String>,
}

/// `MqttConfigSummary`.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct ConfigSummary {
    pub port: u16,
    pub max_connections: u32,
    /// Password mode with at least one user.
    pub auth_enabled: bool,
}

/// `GET /api/v1/mqtt-broker/status`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct BrokerStatus {
    /// `running`, `stopped`, `starting`, or the error text.
    pub status: String,
    pub connected_clients: usize,
    pub clients: Vec<ConnectedClient>,
    /// `null` until the broker has a configuration (none in the server's
    /// file and no `PUT /config` yet).
    pub config: Option<ConfigSummary>,
}

/// What the status says, for colour and for the start/stop button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Running,
    Starting,
    Stopped,
    /// The broker failed; the status text says why.
    Failed,
}

impl BrokerStatus {
    pub fn state(&self) -> State {
        match self.status.as_str() {
            "running" => State::Running,
            "starting" => State::Starting,
            "stopped" | "" => State::Stopped,
            _ => State::Failed,
        }
    }

    pub fn running(&self) -> bool {
        self.state() == State::Running
    }

    /// `Running on port 1883`, `Stopped`, … (the web overview's line).
    pub fn headline(&self) -> String {
        match self.state() {
            State::Running => format!(
                "Running on port {}",
                self.config.map_or(1883, |config| config.port)
            ),
            State::Starting => "Starting…".to_owned(),
            State::Stopped => "Stopped".to_owned(),
            State::Failed => format!("Error: {}", self.status),
        }
    }
}

/// `MqttConfigUpdateResponse`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct ConfigUpdated {
    pub message: String,
    pub config: Option<ConfigSummary>,
}

/// `MqttActionStatusResponse` (start, stop).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct ActionStatus {
    pub message: String,
    pub status: String,
}

/// `crate::mqtt_broker::TopicInfo`; payloads are text, or hex when not
/// UTF-8.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct TopicInfo {
    pub topic: String,
    pub last_payload: String,
    /// Unix seconds.
    pub last_update: i64,
    pub message_count: u64,
    pub retain: bool,
}

/// `GET /api/v1/mqtt-broker/topics`: the flat list and the server's tree
/// (`{"a": {"b": {"_leaf": true, "_message_count": 3, …}}}`). The section
/// builds its own tree from `topics` ([`topic_rows`]); `tree` is kept as
/// sent.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Topics {
    pub topics: Vec<TopicInfo>,
    pub tree: Value,
}

/// `crate::mqtt_broker::RecentMessage`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct RecentMessage {
    pub topic: String,
    pub payload: String,
    /// Unix seconds.
    pub timestamp: i64,
    pub retain: bool,
}

/// `MqttTopicMessagesResponse`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct TopicMessages {
    pub topic: String,
    pub messages: Vec<RecentMessage>,
}

/// `MqttMessagesResponse`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Messages {
    pub messages: Vec<RecentMessage>,
}

/// `MqttPublishResponse`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Published {
    pub message: String,
    pub topic: String,
}

// ── Forms ────────────────────────────────────────────────────────────────

/// A broker user in the config form.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserDraft {
    pub username: String,
    pub password: String,
}

/// The config form as typed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigDraft {
    pub port: String,
    pub max_connections: String,
    pub auth: bool,
    pub users: Vec<UserDraft>,
}

impl ConfigDraft {
    /// The form filled from the status (the web form's defaults, 1883 and
    /// 100, before the broker has a configuration).
    pub fn from_status(config: Option<ConfigSummary>) -> Self {
        let config = config.unwrap_or(ConfigSummary {
            port: 1883,
            max_connections: 100,
            auth_enabled: false,
        });
        Self {
            port: config.port.to_string(),
            max_connections: config.max_connections.to_string(),
            auth: config.auth_enabled,
            users: Vec::new(),
        }
    }

    /// `PUT /api/v1/mqtt-broker/config`'s body, as the web form sends it:
    /// port and max connections always, auth `enabled`, and `users` (named
    /// ones only) when auth is on. The server rejects 0 for either number
    /// and refuses while the broker runs; this checks the numbers first.
    pub fn body(&self) -> Result<Value, String> {
        let port: u16 = self
            .port
            .trim()
            .parse()
            .map_err(|_| "Port must be a number from 1 to 65535".to_owned())?;
        if port == 0 {
            return Err("Port must be a number from 1 to 65535".to_owned());
        }
        let max: u32 = self
            .max_connections
            .trim()
            .parse()
            .map_err(|_| "Max connections must be a whole number above 0".to_owned())?;
        if max == 0 {
            return Err("Max connections must be a whole number above 0".to_owned());
        }
        let mut auth = json!({ "enabled": self.auth });
        if self.auth {
            let users: Vec<Value> = self
                .users
                .iter()
                .filter(|user| !user.username.trim().is_empty())
                .map(|user| json!({ "username": user.username.trim(), "password": user.password }))
                .collect();
            auth["users"] = Value::Array(users);
        }
        Ok(json!({ "port": port, "max_connections": max, "auth": auth }))
    }
}

/// `PUT /config`'s body that puts `config` back (auth users cannot be
/// read back, so only the switch is restored).
pub fn restore_body(config: ConfigSummary) -> Value {
    json!({
        "port": config.port,
        "max_connections": config.max_connections,
        "auth": { "enabled": config.auth_enabled },
    })
}

/// `POST /api/v1/mqtt-broker/publish`'s body; the server refuses an empty
/// topic, so does this.
pub fn publish_body(topic: &str, payload: &str) -> Result<Value, String> {
    let topic = topic.trim();
    if topic.is_empty() {
        return Err("Enter a topic".to_owned());
    }
    if topic.contains(['+', '#']) {
        return Err("A published topic cannot hold the wildcards + or #".to_owned());
    }
    Ok(json!({ "topic": topic, "payload": payload }))
}

/// Up to eight known topics that start with what is typed (the web
/// publish form's suggestions).
pub fn suggestions<'a>(topics: &'a [TopicInfo], typed: &str) -> Vec<&'a str> {
    let typed = typed.trim().to_lowercase();
    if typed.is_empty() {
        return Vec::new();
    }
    topics
        .iter()
        .map(|info| info.topic.as_str())
        .filter(|topic| topic.to_lowercase().starts_with(&typed) && topic.len() > typed.len())
        .take(8)
        .collect()
}

// ── Topic tree ───────────────────────────────────────────────────────────

/// One line of the topic list: a folder (a level with children) or a
/// topic (a leaf that carries messages). A topic can be a folder too
/// (`a` and `a/b` both published).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicRow {
    /// The full path to this level: `site/inverter/power`.
    pub path: String,
    /// The last level: `power`.
    pub name: String,
    pub depth: usize,
    pub has_children: bool,
    pub expanded: bool,
    /// Its index in `topics` when it is a published topic.
    pub topic: Option<usize>,
}

#[derive(Default)]
struct Node {
    children: BTreeMap<String, Node>,
    topic: Option<usize>,
}

/// The rows to draw. With a `filter`, the matching topics flat (as the web
/// client's filtered list); else the tree, `expanded` paths open.
pub fn topic_rows(
    topics: &[TopicInfo],
    filter: &str,
    expanded: &BTreeSet<String>,
) -> Vec<TopicRow> {
    let filter = filter.trim().to_lowercase();
    if !filter.is_empty() {
        let mut rows: Vec<TopicRow> = topics
            .iter()
            .enumerate()
            .filter(|(_, info)| info.topic.to_lowercase().contains(&filter))
            .map(|(at, info)| TopicRow {
                path: info.topic.clone(),
                name: info.topic.clone(),
                depth: 0,
                has_children: false,
                expanded: false,
                topic: Some(at),
            })
            .collect();
        rows.sort_by(|a, b| a.path.cmp(&b.path));
        return rows;
    }
    let mut root = Node::default();
    for (at, info) in topics.iter().enumerate() {
        let mut node = &mut root;
        for part in info.topic.split('/') {
            node = node.children.entry(part.to_owned()).or_default();
        }
        node.topic = Some(at);
    }
    let mut rows = Vec::new();
    flatten(&root, "", 0, expanded, &mut rows);
    rows
}

fn flatten(
    node: &Node,
    prefix: &str,
    depth: usize,
    expanded: &BTreeSet<String>,
    rows: &mut Vec<TopicRow>,
) {
    for (name, child) in &node.children {
        let path = if depth == 0 {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let open = expanded.contains(&path);
        rows.push(TopicRow {
            path: path.clone(),
            name: name.clone(),
            depth,
            has_children: !child.children.is_empty(),
            expanded: open,
            topic: child.topic,
        });
        if open {
            flatten(child, &path, depth + 1, expanded, rows);
        }
    }
}

/// A payload for reading: JSON pretty-printed (and flagged), else as sent.
pub fn pretty_payload(payload: &str) -> (String, bool) {
    match serde_json::from_str::<Value>(payload) {
        Ok(value @ (Value::Object(_) | Value::Array(_))) => (
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| payload.to_owned()),
            true,
        ),
        _ => (payload.to_owned(), false),
    }
}

/// Messages whose topic has `filter` in it (the message log's filter).
pub fn filter_messages<'a>(messages: &'a [RecentMessage], filter: &str) -> Vec<&'a RecentMessage> {
    let filter = filter.trim().to_lowercase();
    messages
        .iter()
        .filter(|message| filter.is_empty() || message.topic.to_lowercase().contains(&filter))
        .collect()
}

// ── Calls ────────────────────────────────────────────────────────────────

impl VoltrusApi {
    pub async fn mqtt_status(&self) -> Result<BrokerStatus> {
        self.get_json(STATUS).await
    }

    pub async fn mqtt_update_config(&self, body: &Value) -> Result<ConfigUpdated> {
        self.send_json(Method::PUT, CONFIG, body).await
    }

    pub async fn mqtt_start(&self) -> Result<ActionStatus> {
        self.send_empty(Method::POST, START).await
    }

    pub async fn mqtt_stop(&self) -> Result<ActionStatus> {
        self.send_empty(Method::POST, STOP).await
    }

    pub async fn mqtt_topics(&self) -> Result<Topics> {
        self.get_json(TOPICS).await
    }

    pub async fn mqtt_topic_messages(&self, topic: &str) -> Result<TopicMessages> {
        self.get_json(&topic_messages_path(topic)).await
    }

    /// The newest `limit` messages over every topic (the server's default
    /// is 100).
    pub async fn mqtt_messages(&self, limit: usize) -> Result<Messages> {
        self.get_json(&format!("{MESSAGES}?limit={limit}")).await
    }

    pub async fn mqtt_publish(&self, body: &Value) -> Result<Published> {
        self.send_json(Method::POST, PUBLISH, body).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_decodes_as_get_status_sends_it() {
        let running = r#"{
            "status": "running", "connected_clients": 1,
            "clients": [{"client_id": "sim-1", "connected_at": 1761390000,
                         "subscriptions": ["site/#"]}],
            "config": {"port": 1884, "max_connections": 50, "auth_enabled": true}
        }"#;
        let status: BrokerStatus = serde_json::from_str(running).unwrap();
        assert!(status.running());
        assert_eq!(status.headline(), "Running on port 1884");
        assert_eq!(status.clients[0].subscriptions, ["site/#"]);
        assert_eq!(status.config.unwrap().max_connections, 50);
        // Never configured: config is null.
        let fresh: BrokerStatus = serde_json::from_str(
            r#"{"status": "stopped", "connected_clients": 0, "clients": [], "config": null}"#,
        )
        .unwrap();
        assert_eq!(fresh.state(), State::Stopped);
        assert_eq!(fresh.config, None);
        // An error is the message itself.
        let failed: BrokerStatus = serde_json::from_str(
            r#"{"status": "bind failed: address in use", "connected_clients": 0, "clients": [], "config": null}"#,
        )
        .unwrap();
        assert_eq!(failed.state(), State::Failed);
        assert_eq!(failed.headline(), "Error: bind failed: address in use");
    }

    #[test]
    fn the_other_answers_decode() {
        let updated: ConfigUpdated = serde_json::from_str(
            r#"{"message": "Configuration updated. Start the broker to apply changes.",
                "config": {"port": 1883, "max_connections": 100, "auth_enabled": false}}"#,
        )
        .unwrap();
        assert_eq!(updated.config.unwrap().port, 1883);
        let action: ActionStatus =
            serde_json::from_str(r#"{"message": "MQTT broker started", "status": "running"}"#)
                .unwrap();
        assert_eq!(action.status, "running");
        let topics: Topics = serde_json::from_str(
            r#"{"topics": [{"topic": "site/inv1/power", "last_payload": "{\"kw\":4.2}",
                            "last_update": 1761390000, "message_count": 3, "retain": false}],
                "tree": {"site": {"inv1": {"power": {"_leaf": true, "_message_count": 3,
                                                       "_last_update": 1761390000}}}}}"#,
        )
        .unwrap();
        assert_eq!(topics.topics[0].message_count, 3);
        assert!(
            topics.tree["site"]["inv1"]["power"]["_leaf"]
                .as_bool()
                .unwrap()
        );
        let messages: TopicMessages = serde_json::from_str(
            r#"{"topic": "a/b", "messages": [{"topic": "a/b", "payload": "1",
                "timestamp": 1761390000, "retain": true}]}"#,
        )
        .unwrap();
        assert!(messages.messages[0].retain);
        let all: Messages = serde_json::from_str(r#"{"messages": []}"#).unwrap();
        assert!(all.messages.is_empty());
        let published: Published =
            serde_json::from_str(r#"{"message": "Published", "topic": "a/b"}"#).unwrap();
        assert_eq!(published.topic, "a/b");
    }

    #[test]
    fn the_config_form_validates_and_builds_the_web_body() {
        let mut draft = ConfigDraft::from_status(None);
        assert_eq!(
            (draft.port.as_str(), draft.max_connections.as_str()),
            ("1883", "100")
        );
        assert_eq!(
            draft.body().unwrap(),
            json!({"port": 1883, "max_connections": 100, "auth": {"enabled": false}})
        );
        draft.auth = true;
        draft.users = vec![
            UserDraft {
                username: " plc ".into(),
                password: "secret".into(),
            },
            UserDraft::default(),
        ];
        assert_eq!(
            draft.body().unwrap()["auth"],
            json!({"enabled": true, "users": [{"username": "plc", "password": "secret"}]})
        );
        draft.port = "0".into();
        assert!(draft.body().is_err());
        draft.port = "70000".into();
        assert!(draft.body().is_err());
        draft.port = "1884".into();
        draft.max_connections = "0".into();
        assert!(draft.body().is_err());
        let back = restore_body(ConfigSummary {
            port: 1883,
            max_connections: 100,
            auth_enabled: false,
        });
        assert_eq!(back["auth"], json!({"enabled": false}));
    }

    #[test]
    fn publish_needs_a_topic_without_wildcards() {
        assert!(publish_body("  ", "x").is_err());
        assert!(publish_body("a/#", "x").is_err());
        assert_eq!(
            publish_body(" a/b ", "{\"v\":1}").unwrap(),
            json!({"topic": "a/b", "payload": "{\"v\":1}"})
        );
    }

    fn topic(name: &str) -> TopicInfo {
        TopicInfo {
            topic: name.into(),
            ..TopicInfo::default()
        }
    }

    #[test]
    fn the_topic_tree_flattens_with_open_folders() {
        let topics = vec![
            topic("site/inv1/power"),
            topic("site/inv2/power"),
            topic("site"),
            topic("x"),
        ];
        let closed = topic_rows(&topics, "", &BTreeSet::new());
        let paths: Vec<&str> = closed.iter().map(|row| row.path.as_str()).collect();
        assert_eq!(paths, ["site", "x"]);
        assert!(closed[0].has_children && closed[0].topic == Some(2));
        let open: BTreeSet<String> = ["site".to_owned(), "site/inv1".to_owned()].into();
        let rows = topic_rows(&topics, "", &open);
        let paths: Vec<(&str, usize)> = rows.iter().map(|r| (r.path.as_str(), r.depth)).collect();
        assert_eq!(
            paths,
            [
                ("site", 0),
                ("site/inv1", 1),
                ("site/inv1/power", 2),
                ("site/inv2", 1),
                ("x", 0)
            ]
        );
        assert_eq!(rows[2].topic, Some(0));
        assert_eq!(rows[2].name, "power");
        // A filter lists matching topics flat.
        let found = topic_rows(&topics, "INV2", &BTreeSet::new());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, "site/inv2/power");
        assert_eq!(
            suggestions(&topics, "site/"),
            ["site/inv1/power", "site/inv2/power"]
        );
        assert!(suggestions(&topics, "").is_empty());
    }

    #[test]
    fn payloads_and_paths() {
        let (text, json) = pretty_payload("{\"a\":1}");
        assert!(json && text.contains('\n'));
        assert_eq!(pretty_payload("42"), ("42".to_owned(), false));
        assert_eq!(
            topic_messages_path("site/inv 1"),
            "/api/v1/mqtt-broker/topics/site%2Finv%201/messages"
        );
        let messages = vec![
            RecentMessage {
                topic: "a/b".into(),
                ..RecentMessage::default()
            },
            RecentMessage {
                topic: "c".into(),
                ..RecentMessage::default()
            },
        ];
        assert_eq!(filter_messages(&messages, "A/").len(), 1);
        assert_eq!(filter_messages(&messages, "").len(), 2);
    }
}
