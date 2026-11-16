use dashmap::DashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use super::message::Message;

/// A message a `rate_limit` node in `keep_latest` mode held back. The engine
/// sends it on (port 0 of that node) once `release_at` has passed.
#[derive(Debug, Clone)]
pub struct ParkedMessage {
    pub flow_id: String,
    pub node_id: String,
    pub message: Message,
    pub release_at: Instant,
}

/// Per-node memory that outlives a single run (rolling windows, the last value
/// an `on_change` node saw, rate-limit windows). Clones share the same maps.
#[derive(Clone, Default)]
pub struct FlowState {
    rolling_windows: Arc<DashMap<String, VecDeque<f64>>>,
    last_values: Arc<DashMap<String, serde_json::Value>>,
    rate_windows: Arc<DashMap<String, VecDeque<Instant>>>,
    parked: Arc<DashMap<String, ParkedMessage>>,
}

impl FlowState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_value(&self, key: &str, value: f64, window_size: usize) {
        let mut entry = self.rolling_windows.entry(key.to_string()).or_default();
        entry.push_back(value);
        while entry.len() > window_size {
            entry.pop_front();
        }
    }

    pub fn rolling_avg(&self, key: &str) -> Option<f64> {
        self.rolling_windows.get(key).map(|entry| {
            let sum: f64 = entry.iter().sum();
            sum / entry.len() as f64
        })
    }

    /// The value an `on_change` node last let through.
    pub fn last_value(&self, key: &str) -> Option<serde_json::Value> {
        self.last_values.get(key).map(|v| v.clone())
    }

    pub fn set_last_value(&self, key: &str, value: serde_json::Value) {
        self.last_values.insert(key.to_string(), value);
    }

    /// Record a pass through a rate-limited node if fewer than `max` passed
    /// within `window`. Returns `Ok(())` when the message may pass, otherwise
    /// `Err(when)` with the instant the oldest pass leaves the window.
    pub fn rate_try_pass(
        &self,
        key: &str,
        max: usize,
        window: std::time::Duration,
        now: Instant,
    ) -> Result<(), Instant> {
        let mut entry = self.rate_windows.entry(key.to_string()).or_default();
        while entry
            .front()
            .is_some_and(|t| now.duration_since(*t) >= window)
        {
            entry.pop_front();
        }
        if entry.len() < max {
            entry.push_back(now);
            Ok(())
        } else {
            Err(*entry.front().expect("non-empty when full") + window)
        }
    }

    /// Hold `message` for later; replaces anything already parked on that node.
    pub fn park(&self, parked: ParkedMessage) {
        let key = format!("{}/{}", parked.flow_id, parked.node_id);
        self.parked.insert(key, parked);
    }

    /// Remove and return the parked messages whose release time has come.
    pub fn take_due(&self, now: Instant) -> Vec<ParkedMessage> {
        let due: Vec<String> = self
            .parked
            .iter()
            .filter(|e| e.release_at <= now)
            .map(|e| e.key().clone())
            .collect();
        due.into_iter()
            .filter_map(|k| self.parked.remove(&k).map(|(_, v)| v))
            .collect()
    }

    /// Forget everything a flow's nodes remembered (the flow was edited or removed).
    pub fn clear_flow(&self, flow_id: &str) {
        let prefix = format!("{flow_id}/");
        self.last_values.retain(|k, _| !k.starts_with(&prefix));
        self.rate_windows.retain(|k, _| !k.starts_with(&prefix));
        self.parked.retain(|k, _| !k.starts_with(&prefix));
    }
}
