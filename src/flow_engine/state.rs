use dashmap::DashMap;
use std::collections::VecDeque;

#[derive(Clone)]
pub struct FlowState {
    rolling_windows: DashMap<String, VecDeque<f64>>,
}

impl FlowState {
    pub fn new() -> Self {
        Self {
            rolling_windows: DashMap::new(),
        }
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
}
