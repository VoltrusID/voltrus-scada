use crate::cache::DeviceCache;
use crate::flow_engine::{
    cron::Cron,
    executor::{self, ExecEnv, ExecResult},
    graph::{FlowGraph, TRIGGER_ID},
    message::Message,
    run_log,
    state::FlowState,
    trigger::TriggerConfig,
};
use crate::storage::Storage;
use crate::ws_types::ServerMessage;
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};

#[cfg(feature = "mqtt-broker")]
use crate::mqtt_broker::{MqttEvent, TopicTracker};
#[cfg(feature = "mqtt-broker")]
use std::sync::Mutex;

/// A flow as the engine runs it.
#[derive(Debug, Clone)]
pub struct Flow {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub trigger: TriggerConfig,
    pub graph: FlowGraph,
}

impl Flow {
    /// Parse a stored flow (`nodes` v1 or v2).
    pub fn from_db(db: &crate::models::Flow) -> Result<Self, String> {
        Ok(Self {
            id: db.id.clone(),
            name: db.name.clone(),
            enabled: db.enabled,
            trigger: TriggerConfig::from_value(serde_json::Value::String(
                db.trigger_config.clone(),
            ))?,
            graph: FlowGraph::parse(&db.nodes)?,
        })
    }
}

/// Why a webhook or manual run did not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FireError {
    NotFound,
    Disabled,
    WrongTrigger,
    /// A run of the flow is in progress.
    Busy,
    /// This node is the passive one of a redundant pair.
    Passive,
}

impl FireError {
    pub fn message(&self) -> &'static str {
        match self {
            FireError::NotFound => "flow not found",
            FireError::Disabled => "flow is stopped",
            FireError::WrongTrigger => "the flow's trigger is not a webhook",
            FireError::Busy => "a run of this flow is in progress",
            FireError::Passive => "this node is passive; flows run on the active node",
        }
    }
}

/// Where a data source's value lives in the device cache.
#[derive(Debug, Clone)]
struct TagRef {
    device_id: String,
    metric: String,
    key: String,
}

/// Everything the engine's tasks share.
#[derive(Clone)]
struct Shared {
    flows: Arc<RwLock<HashMap<String, Flow>>>,
    state: FlowState,
    storage: Storage,
    cache: DeviceCache,
    smtp: Arc<RwLock<Option<crate::models::SmtpConfig>>>,
    in_flight: Arc<DashMap<String, ()>>,
    last_fired: Arc<DashMap<String, std::time::Instant>>,
    last_values: Arc<DashMap<String, serde_json::Value>>,
    /// data source id -> where to read it, for tag_change triggers.
    tags: Arc<DashMap<i64, TagRef>>,
    /// (flow id, data source id) -> value the trigger last fired on (or first saw).
    tag_last: Arc<DashMap<(String, i64), f64>>,
    /// flow id -> the minute (Unix minutes) a schedule trigger last fired.
    schedule_last: Arc<DashMap<String, i64>>,
}

pub struct FlowEngine {
    shared: Shared,
    ws_manager: Option<Arc<crate::ws_manager::WsManager>>,
    #[cfg(feature = "mqtt-broker")]
    broker_tracker: Arc<Mutex<Option<TopicTracker>>>,
}

impl FlowEngine {
    pub fn new(storage: Storage, cache: DeviceCache) -> Self {
        Self {
            shared: Shared {
                flows: Arc::new(RwLock::new(HashMap::new())),
                state: FlowState::new(),
                storage,
                cache,
                smtp: Arc::new(RwLock::new(None)),
                in_flight: Arc::new(DashMap::new()),
                last_fired: Arc::new(DashMap::new()),
                last_values: Arc::new(DashMap::new()),
                tags: Arc::new(DashMap::new()),
                tag_last: Arc::new(DashMap::new()),
                schedule_last: Arc::new(DashMap::new()),
            },
            ws_manager: None,
            #[cfg(feature = "mqtt-broker")]
            broker_tracker: Arc::new(Mutex::new(None)),
        }
    }

    /// Share the server's SMTP settings with `notify` nodes (the same handle
    /// the settings API updates).
    pub fn with_smtp(mut self, smtp: Arc<RwLock<Option<crate::models::SmtpConfig>>>) -> Self {
        self.shared.smtp = smtp;
        self
    }

    /// Listen to alarm events for flows with an alarm trigger. Set before `spawn`.
    pub fn with_ws_manager(mut self, ws: Arc<crate::ws_manager::WsManager>) -> Self {
        self.ws_manager = Some(ws);
        self
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
        let mut rx = self.shared.cache.subscribe();

        // Clock: interval and schedule triggers, held rate-limit messages, and
        // the tag map refresh.
        {
            let shared = self.shared.clone();
            tokio::spawn(async move {
                let mut last_run: HashMap<String, std::time::Instant> = HashMap::new();
                let mut ticks: u64 = 0;
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    ticks += 1;
                    if ticks.is_multiple_of(30) {
                        refresh_tags(&shared).await;
                    }
                    clock_tick(&shared, &mut last_run).await;
                }
            });
        }

        // Alarm events for alarm triggers.
        if let Some(ws) = &self.ws_manager {
            let mut alarms = ws.subscribe_alarm_events();
            let shared = self.shared.clone();
            tokio::spawn(async move {
                loop {
                    match alarms.recv().await {
                        Ok(event) => on_alarm_event(&shared, event).await,
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("FlowEngine alarm events lagged by {} messages", n);
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
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
            let shared = self.shared.clone();
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
                                fire_matching(&shared, &msg).await;
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

        // Device polls: modbus (device) and tag_change triggers.
        let shared = self.shared.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(snapshot) => {
                        on_tag_changes(&shared, &snapshot).await;
                        let msg = device_snapshot_to_message(snapshot);
                        fire_matching(&shared, &msg).await;
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
        self.update_flow(flow).await;
    }

    pub async fn remove_flow(&self, flow_id: &str) {
        self.shared.flows.write().await.remove(flow_id);
        self.forget(flow_id);
    }

    pub async fn get_flow(&self, flow_id: &str) -> Option<Flow> {
        let flows = self.shared.flows.read().await;
        flows.get(flow_id).cloned()
    }

    pub async fn list_flows(&self) -> Vec<Flow> {
        let flows = self.shared.flows.read().await;
        flows.values().cloned().collect()
    }

    pub async fn update_flow(&self, flow: Flow) {
        let watches_tags = matches!(flow.trigger, TriggerConfig::TagChange { .. });
        self.forget(&flow.id);
        self.shared
            .flows
            .write()
            .await
            .insert(flow.id.clone(), flow);
        if watches_tags {
            refresh_tags(&self.shared).await;
        }
    }

    /// Drop what the flow's nodes and triggers remembered (it changed or went away).
    fn forget(&self, flow_id: &str) {
        self.shared.state.clear_flow(flow_id);
        self.shared.tag_last.retain(|(f, _), _| f != flow_id);
        self.shared.last_values.remove(flow_id);
        self.shared.schedule_last.remove(flow_id);
    }

    /// Fire a webhook flow with `payload`. The run happens in the background.
    pub async fn fire_webhook(
        &self,
        flow_id: &str,
        payload: serde_json::Value,
    ) -> Result<(), FireError> {
        if !crate::node_role::is_active() {
            return Err(FireError::Passive);
        }
        let flow = self.get_flow(flow_id).await.ok_or(FireError::NotFound)?;
        if !matches!(flow.trigger, TriggerConfig::Webhook {}) {
            return Err(FireError::WrongTrigger);
        }
        if !flow.enabled {
            return Err(FireError::Disabled);
        }
        let mut msg = Message::new(payload);
        msg.metadata.insert("source".into(), "webhook".into());
        if !gate_fire(
            &flow,
            &msg,
            &self.shared.in_flight,
            &self.shared.last_fired,
            &self.shared.last_values,
        ) {
            return Err(FireError::Busy);
        }
        spawn_runs(&self.shared, flow, vec![(TRIGGER_ID.to_string(), msg)]);
        Ok(())
    }

    /// Run `flow` now with `msg` (run button / manual trigger), whatever its
    /// trigger and whether or not it is started, and wait for the result.
    pub async fn run_now(&self, flow: Flow, mut msg: Message) -> Result<ExecResult, FireError> {
        if !crate::node_role::is_active() {
            return Err(FireError::Passive);
        }
        msg.metadata
            .entry("source".into())
            .or_insert_with(|| "manual".into());
        if self.shared.in_flight.contains_key(&flow.id) {
            return Err(FireError::Busy);
        }
        self.shared.in_flight.insert(flow.id.clone(), ());
        let result = run_logged(&self.shared, &flow, TRIGGER_ID, 0, msg).await;
        self.shared.in_flight.remove(&flow.id);
        Ok(result)
    }
}

/// Run `flow` once with `msg` and record the run, without an engine (the API
/// uses this when the engine is not running).
pub async fn run_once(
    storage: &Storage,
    cache: &DeviceCache,
    smtp: Option<crate::models::SmtpConfig>,
    flow: &Flow,
    msg: Message,
) -> ExecResult {
    let env = ExecEnv {
        flow_id: flow.id.clone(),
        state: FlowState::new(),
        storage: storage.clone(),
        cache: cache.clone(),
        smtp,
        dry_run: false,
    };
    execute_logged(&env, flow, TRIGGER_ID, 0, msg).await
}

async fn run_logged(
    shared: &Shared,
    flow: &Flow,
    start: &str,
    port: usize,
    msg: Message,
) -> ExecResult {
    let env = ExecEnv {
        flow_id: flow.id.clone(),
        state: shared.state.clone(),
        storage: shared.storage.clone(),
        cache: shared.cache.clone(),
        smtp: shared.smtp.read().await.clone(),
        dry_run: false,
    };
    execute_logged(&env, flow, start, port, msg).await
}

async fn execute_logged(
    env: &ExecEnv,
    flow: &Flow,
    start: &str,
    port: usize,
    msg: Message,
) -> ExecResult {
    let started = std::time::Instant::now();
    let result = executor::execute(&flow.graph, start, port, msg, env).await;
    run_log::log_run_traced(
        &env.storage,
        &flow.id,
        started.elapsed().as_millis() as u64,
        result.ok,
        result.error.clone(),
        if result.ok {
            result.last_payload.clone()
        } else {
            None
        },
        Some(executor::truncate_trace(&result.trace)),
    )
    .await;
    result
}

/// Run each `(start node, message)` in turn in the background, then free the
/// flow for its next run. The caller has passed `gate_fire`.
fn spawn_runs(shared: &Shared, flow: Flow, runs: Vec<(String, Message)>) {
    let shared = shared.clone();
    tokio::spawn(async move {
        // Flows act on devices and on the outside world: only the active node
        // of a redundant pair runs them.
        if crate::node_role::is_active() {
            for (start, msg) in runs {
                run_logged(&shared, &flow, &start, 0, msg).await;
            }
        }
        shared.in_flight.remove(&flow.id);
    });
}

/// Fire every enabled flow whose trigger matches a device or MQTT message.
async fn fire_matching(shared: &Shared, msg: &Message) {
    let flows = shared.flows.read().await;
    for flow in flows.values() {
        if flow.enabled
            && flow.trigger.matches(msg)
            && gate_fire(
                flow,
                msg,
                &shared.in_flight,
                &shared.last_fired,
                &shared.last_values,
            )
        {
            spawn_runs(shared, flow.clone(), vec![(TRIGGER_ID.into(), msg.clone())]);
        }
    }
}

/// Re-read where the data sources of tag_change triggers live.
async fn refresh_tags(shared: &Shared) {
    let ids: Vec<i64> = {
        let flows = shared.flows.read().await;
        let mut ids: Vec<i64> = flows
            .values()
            .filter_map(|f| match &f.trigger {
                TriggerConfig::TagChange {
                    data_source_ids, ..
                } => Some(data_source_ids.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    shared.tags.retain(|id, _| ids.binary_search(id).is_ok());
    for id in ids {
        match shared.storage.get_data_source_by_id(id).await {
            Ok(Some(ds)) => {
                let metric = crate::api::data::metric_name(&ds.key, &ds.device_id).to_string();
                shared.tags.insert(
                    id,
                    TagRef {
                        device_id: ds.device_id,
                        metric,
                        key: ds.key,
                    },
                );
            }
            Ok(None) => {
                shared.tags.remove(&id);
            }
            Err(e) => tracing::warn!("FlowEngine: reading data source {id} failed: {e}"),
        }
    }
}

/// Fire tag_change flows for the tags of this device that moved past their deadband.
async fn on_tag_changes(shared: &Shared, snap: &crate::models::DeviceSnapshot) {
    let flows = shared.flows.read().await;
    for flow in flows.values() {
        let TriggerConfig::TagChange {
            data_source_ids,
            deadband,
        } = &flow.trigger
        else {
            continue;
        };
        if !flow.enabled {
            continue;
        }
        let deadband = deadband.unwrap_or(0.0);
        let mut changes: Vec<(i64, f64, Option<f64>, Message)> = Vec::new();
        for id in data_source_ids {
            let Some(tag) = shared.tags.get(id).map(|t| t.clone()) else {
                continue;
            };
            if tag.device_id != snap.device_id {
                continue;
            }
            let Some(value) = snap.metrics.get(&tag.metric).copied() else {
                continue;
            };
            let key = (flow.id.clone(), *id);
            let previous = shared.tag_last.get(&key).map(|v| *v);
            let Some(prev) = previous else {
                // First value: the baseline, not a change.
                shared.tag_last.insert(key, value);
                continue;
            };
            let changed = if value.is_nan() || prev.is_nan() {
                value.is_nan() != prev.is_nan()
            } else {
                (value - prev).abs() > deadband
            };
            if !changed {
                continue;
            }
            let payload = serde_json::json!({
                "data_source_id": id,
                "key": tag.key,
                "device_id": tag.device_id,
                "metric": tag.metric,
                "value": value,
                "previous": prev,
                "quality": snap.quality_of(&tag.metric).as_str(),
                "ts": snap.metric_ts(&tag.metric),
            });
            let mut msg = Message::new(payload);
            msg.metadata.insert("source".into(), "tag_change".into());
            changes.push((*id, value, Some(prev), msg));
        }
        if changes.is_empty() {
            continue;
        }
        if !gate_fire(
            flow,
            &changes[0].3,
            &shared.in_flight,
            &shared.last_fired,
            &shared.last_values,
        ) {
            // Busy: keep the old baselines so the change fires on a later poll.
            continue;
        }
        let mut runs = Vec::with_capacity(changes.len());
        for (id, value, _, msg) in changes {
            shared.tag_last.insert((flow.id.clone(), id), value);
            runs.push((TRIGGER_ID.to_string(), msg));
        }
        spawn_runs(shared, flow.clone(), runs);
    }
}

/// An alarm message as an alarm-trigger event: (event, alarm id, payload, severity if known).
fn alarm_event(
    msg: &ServerMessage,
) -> Option<(&'static str, i64, serde_json::Value, Option<String>)> {
    match msg {
        ServerMessage::Alarm {
            alarm_id,
            tag_id,
            severity,
            message,
            value,
            ts,
        } => Some((
            "raised",
            *alarm_id,
            serde_json::json!({
                "event": "raised", "alarm_id": alarm_id, "source_key": tag_id,
                "severity": severity, "message": message, "value": value,
                "state": "active", "actor": null, "ts": ts,
            }),
            Some(severity.clone()),
        )),
        ServerMessage::AlarmUpdate {
            alarm_id,
            tag_id,
            event,
            state,
            actor,
            ts,
        } => {
            let event = match event.as_str() {
                "resolved" => "cleared",
                "acknowledged" => "acknowledged",
                _ => return None,
            };
            Some((
                event,
                *alarm_id,
                serde_json::json!({
                    "event": event, "alarm_id": alarm_id, "source_key": tag_id,
                    "state": state, "actor": actor, "ts": ts,
                }),
                None,
            ))
        }
        _ => None,
    }
}

async fn on_alarm_event(shared: &Shared, event: ServerMessage) {
    let Some((kind, alarm_id, mut payload, mut severity)) = alarm_event(&event) else {
        return;
    };
    let source_key = payload["source_key"].as_str().unwrap_or("").to_string();
    let candidates: Vec<Flow> = {
        let flows = shared.flows.read().await;
        flows
            .values()
            .filter(|f| f.enabled)
            .filter(|f| match &f.trigger {
                TriggerConfig::Alarm {
                    on, source_key: sk, ..
                } => {
                    (on == "any" || on == kind)
                        && sk
                            .as_deref()
                            .filter(|s| !s.is_empty())
                            .is_none_or(|s| s == source_key)
                }
                _ => false,
            })
            .cloned()
            .collect()
    };
    if candidates.is_empty() {
        return;
    }
    if severity.is_none() {
        severity = shared
            .storage
            .get_alarm_severity(alarm_id)
            .await
            .ok()
            .flatten();
    }
    payload["severity"] = serde_json::json!(severity);
    for flow in candidates {
        if let TriggerConfig::Alarm {
            severity: Some(want),
            ..
        } = &flow.trigger
            && !want.is_empty()
            && severity.as_deref() != Some(want.as_str())
        {
            continue;
        }
        let mut msg = Message::new(payload.clone());
        msg.metadata.insert("source".into(), "alarm".into());
        if gate_fire(
            &flow,
            &msg,
            &shared.in_flight,
            &shared.last_fired,
            &shared.last_values,
        ) {
            spawn_runs(shared, flow, vec![(TRIGGER_ID.into(), msg)]);
        }
    }
}

/// One second of the clock: interval and schedule triggers, then held messages.
async fn clock_tick(shared: &Shared, last_run: &mut HashMap<String, std::time::Instant>) {
    let now = std::time::Instant::now();
    let now_secs = crate::timeutil::now_secs();
    let minute = now_secs.div_euclid(60);
    let local = crate::flow_engine::cron::local_time(now_secs);
    {
        let flows = shared.flows.read().await;
        for flow in flows.values() {
            if !flow.enabled {
                continue;
            }
            let due = match &flow.trigger {
                TriggerConfig::Timer { interval_ms } => {
                    let due = last_run.get(&flow.id).is_none_or(|last| {
                        now.duration_since(*last).as_millis() as u64 >= *interval_ms
                    });
                    if due {
                        last_run.insert(flow.id.clone(), now);
                    }
                    due
                }
                TriggerConfig::Schedule { cron } => {
                    let fired_this_minute = shared
                        .schedule_last
                        .get(&flow.id)
                        .is_some_and(|m| *m == minute);
                    let due =
                        !fired_this_minute && Cron::parse(cron).is_ok_and(|c| c.matches(local));
                    if due {
                        shared.schedule_last.insert(flow.id.clone(), minute);
                    }
                    due
                }
                _ => false,
            };
            if !due {
                continue;
            }
            let mut msg = Message::new(serde_json::json!({"timestamp": now_secs}));
            msg.metadata
                .insert("source".into(), flow.trigger.type_name().into());
            if gate_fire(
                flow,
                &msg,
                &shared.in_flight,
                &shared.last_fired,
                &shared.last_values,
            ) {
                spawn_runs(shared, flow.clone(), vec![(TRIGGER_ID.into(), msg)]);
            }
        }
    }
    release_parked(shared, now).await;
}

/// Send on the messages rate-limit nodes held whose time has come.
async fn release_parked(shared: &Shared, now: std::time::Instant) {
    for parked in shared.state.take_due(now) {
        let flow = shared.flows.read().await.get(&parked.flow_id).cloned();
        let Some(flow) = flow.filter(|f| f.enabled) else {
            continue;
        };
        if shared.in_flight.contains_key(&flow.id) {
            // Try again on the next tick.
            shared.state.park(parked);
            continue;
        }
        // The released message counts against the node's window like any other.
        if let Some(crate::flow_engine::node::NodeConfig::RateLimit {
            max, duration_ms, ..
        }) = flow.graph.node(&parked.node_id).map(|n| &n.config)
            && let Err(release_at) = shared.state.rate_try_pass(
                &format!("{}/{}", flow.id, parked.node_id),
                *max as usize,
                std::time::Duration::from_millis(*duration_ms),
                now,
            )
        {
            shared.state.park(crate::flow_engine::state::ParkedMessage {
                release_at,
                ..parked
            });
            continue;
        }
        shared.in_flight.insert(flow.id.clone(), ());
        spawn_runs(shared, flow, vec![(parked.node_id, parked.message)]);
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
        _ => None,
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

#[cfg(test)]
mod gate_fire_tests {
    use super::*;

    fn flow(id: &str, trigger: TriggerConfig) -> Flow {
        Flow {
            id: id.to_string(),
            name: id.to_string(),
            enabled: true,
            trigger,
            graph: FlowGraph::default(),
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
}

#[cfg(test)]
mod trigger_source_tests {
    use super::*;
    use serde_json::json;

    async fn engine_with(storage: Storage, cache: DeviceCache) -> FlowEngine {
        let ws = Arc::new(crate::ws_manager::WsManager::new());
        let engine = FlowEngine::new(storage, cache).with_ws_manager(ws);
        engine.spawn().await;
        engine
    }

    fn debug_flow(id: &str, trigger: Value) -> Flow {
        Flow {
            id: id.into(),
            name: id.into(),
            enabled: true,
            trigger: TriggerConfig::from_value(trigger).unwrap(),
            graph: FlowGraph::from_value(json!({"version": 2,
                "nodes": [{"id": "d", "type": "debug"}],
                "edges": [{"from": "trigger", "to": "d"}]}))
            .unwrap(),
        }
    }

    use serde_json::Value;

    /// Wait until the flow has `n` runs logged (or give up after 3 s).
    async fn runs(storage: &Storage, flow: &str, n: usize) -> Vec<crate::models::FlowRun> {
        for _ in 0..60 {
            let runs = storage.list_flow_runs(flow, 50).await.unwrap();
            if runs.len() >= n {
                return runs;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        storage.list_flow_runs(flow, 50).await.unwrap()
    }

    #[tokio::test]
    async fn tag_change_fires_on_changes_past_the_deadband() {
        let storage = Storage::new_in_memory().unwrap();
        storage.create_device("plc-1", "PLC 1", 0).await.unwrap();
        let id = storage
            .create_data_source(
                "plc-1.temp",
                "Temp",
                "C",
                "",
                0.0,
                100.0,
                0.0,
                0.0,
                0.0,
                0,
                2,
                "modbus",
                "{}",
                "plc-1",
                None,
                "line",
            )
            .await
            .unwrap();
        let cache = DeviceCache::new();
        let engine = engine_with(storage.clone(), cache.clone()).await;
        engine
            .add_flow(debug_flow(
                "tc",
                json!({"type": "tag_change", "data_source_ids": [id], "deadband": 1.0}),
            ))
            .await;
        let put = |v: f64| {
            let cache = cache.clone();
            async move {
                cache
                    .update("plc-1", HashMap::from([("temp".to_string(), v)]), false)
                    .await;
                // Let the engine take the snapshot and finish the run.
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        };
        put(20.0).await; // baseline
        put(20.5).await; // within the deadband
        assert_eq!(storage.list_flow_runs("tc", 50).await.unwrap().len(), 0);
        put(21.5).await; // 1.5 past the baseline
        let r = runs(&storage, "tc", 1).await;
        assert_eq!(r.len(), 1);
        let trace = r[0].trace.as_ref().unwrap();
        let payload = &trace[0]["input"];
        assert_eq!(payload["data_source_id"], id);
        assert_eq!(payload["value"], 21.5);
        assert_eq!(payload["previous"], 20.0);
        assert_eq!(payload["quality"], "good");
        // Another device's update does not fire it.
        cache
            .update("plc-2", HashMap::from([("temp".to_string(), 99.0)]), false)
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(storage.list_flow_runs("tc", 50).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn alarm_trigger_fires_on_matching_events() {
        let storage = Storage::new_in_memory().unwrap();
        let ws = Arc::new(crate::ws_manager::WsManager::new());
        let engine =
            FlowEngine::new(storage.clone(), DeviceCache::new()).with_ws_manager(ws.clone());
        engine.spawn().await;
        engine
            .add_flow(debug_flow(
                "raised",
                json!({"type": "alarm", "on": "raised", "severity": "critical"}),
            ))
            .await;
        engine
            .add_flow(debug_flow(
                "cleared",
                json!({"type": "alarm", "on": "cleared", "source_key": "plc.temp"}),
            ))
            .await;
        let raise = |severity: &str| ServerMessage::Alarm {
            alarm_id: 1,
            tag_id: "plc.temp".into(),
            severity: severity.into(),
            message: "hot".into(),
            value: 90.0,
            ts: 0,
        };
        ws.broadcast_alarm(raise("warning"));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            storage
                .list_flow_runs("raised", 5)
                .await
                .unwrap()
                .is_empty()
        );
        ws.broadcast_alarm(raise("critical"));
        let r = runs(&storage, "raised", 1).await;
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].trace.as_ref().unwrap()[0]["input"]["event"], "raised");

        ws.broadcast_alarm_updates(
            "resolved",
            None,
            &[crate::models::AlarmTransition {
                alarm_id: 1,
                source_key: "plc.other".into(),
                state: "resolved".into(),
            }],
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            storage
                .list_flow_runs("cleared", 5)
                .await
                .unwrap()
                .is_empty()
        );
        ws.broadcast_alarm_updates(
            "resolved",
            None,
            &[crate::models::AlarmTransition {
                alarm_id: 1,
                source_key: "plc.temp".into(),
                state: "resolved".into(),
            }],
        );
        let r = runs(&storage, "cleared", 1).await;
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].trace.as_ref().unwrap()[0]["input"]["event"], "cleared");
    }

    #[tokio::test]
    async fn webhook_fires_only_webhook_flows_that_are_started() {
        let storage = Storage::new_in_memory().unwrap();
        let engine = engine_with(storage.clone(), DeviceCache::new()).await;
        engine
            .add_flow(debug_flow("hook", json!({"type": "webhook"})))
            .await;
        engine
            .add_flow(debug_flow("manual", json!({"type": "manual"})))
            .await;
        let mut stopped = debug_flow("stopped", json!({"type": "webhook"}));
        stopped.enabled = false;
        engine.add_flow(stopped).await;

        engine.fire_webhook("hook", json!({"a": 1})).await.unwrap();
        let r = runs(&storage, "hook", 1).await;
        assert_eq!(r[0].trace.as_ref().unwrap()[0]["input"]["a"], 1);
        assert_eq!(
            engine.fire_webhook("manual", json!({})).await,
            Err(FireError::WrongTrigger)
        );
        assert_eq!(
            engine.fire_webhook("stopped", json!({})).await,
            Err(FireError::Disabled)
        );
        assert_eq!(
            engine.fire_webhook("nope", json!({})).await,
            Err(FireError::NotFound)
        );
    }

    #[tokio::test]
    async fn run_now_runs_any_flow_and_logs_it() {
        let storage = Storage::new_in_memory().unwrap();
        let engine = engine_with(storage.clone(), DeviceCache::new()).await;
        let mut f = debug_flow("m", json!({"type": "manual"}));
        f.enabled = false;
        let r = engine
            .run_now(f, Message::new(json!({"x": 1})))
            .await
            .unwrap();
        assert!(r.ok);
        assert_eq!(r.outputs_reached, ["d"]);
        let logged = storage.list_flow_runs("m", 5).await.unwrap();
        assert_eq!(logged.len(), 1);
        assert_eq!(logged[0].trace.as_ref().unwrap()[0]["node_id"], "d");
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
