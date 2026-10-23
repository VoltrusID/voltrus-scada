use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct DeadbandEntry {
    pub deadband_type: DeadbandType,
    pub deadband_value: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeadbandType {
    Percent,
    Absolute,
}

pub struct DeadbandFilter {
    last_values: Arc<RwLock<HashMap<(String, String), f64>>>,
    configs: Arc<RwLock<HashMap<(String, String), DeadbandEntry>>>,
    default_analog_percent: f64,
    absolute_min: f64,
}

impl DeadbandFilter {
    pub fn new() -> Self {
        Self {
            last_values: Arc::new(RwLock::new(HashMap::new())),
            configs: Arc::new(RwLock::new(HashMap::new())),
            default_analog_percent: 0.001,
            absolute_min: 0.001,
        }
    }

    pub async fn set_config(&self, device_id: &str, metric: &str, entry: DeadbandEntry) {
        let mut configs = self.configs.write().await;
        configs.insert((device_id.to_string(), metric.to_string()), entry);
    }

    pub async fn load_configs(&self, entries: Vec<(String, String, DeadbandEntry)>) {
        let mut configs = self.configs.write().await;
        for (device_id, metric, entry) in entries {
            configs.insert((device_id, metric), entry);
        }
    }

    pub async fn should_write(&self, device_id: &str, metric: &str, new_value: f64) -> bool {
        let key = (device_id.to_string(), metric.to_string());
        let last_values = self.last_values.read().await;
        if let Some(&last) = last_values.get(&key) {
            let threshold = self.compute_threshold(device_id, metric, last).await;
            (new_value - last).abs() > threshold
        } else {
            true
        }
    }

    pub async fn record_write(&self, device_id: &str, metric: &str, value: f64) {
        let mut last_values = self.last_values.write().await;
        last_values.insert((device_id.to_string(), metric.to_string()), value);
    }

    pub async fn filter_batch(
        &self,
        records: &[(String, i64, String, f64)],
    ) -> Vec<(String, i64, String, f64)> {
        let mut result = Vec::new();
        let configs = self.configs.read().await;
        let mut last_values = self.last_values.write().await;

        for (device_id, ts, metric, value) in records {
            let key = (device_id.clone(), metric.clone());
            let should_write = match last_values.get(&key) {
                Some(&last) => {
                    let threshold = if let Some(cfg) = configs.get(&key) {
                        match cfg.deadband_type {
                            DeadbandType::Percent => {
                                (cfg.deadband_value * last.abs()).max(self.absolute_min)
                            }
                            DeadbandType::Absolute => cfg.deadband_value,
                        }
                    } else {
                        (self.default_analog_percent * last.abs()).max(self.absolute_min)
                    };
                    (value - last).abs() > threshold
                }
                None => true,
            };

            if should_write {
                last_values.insert(key, *value);
                result.push((device_id.clone(), *ts, metric.clone(), *value));
            }
        }
        result
    }

    async fn compute_threshold(&self, device_id: &str, metric: &str, last_value: f64) -> f64 {
        let configs = self.configs.read().await;
        let key = (device_id.to_string(), metric.to_string());
        if let Some(cfg) = configs.get(&key) {
            match cfg.deadband_type {
                DeadbandType::Percent => {
                    (cfg.deadband_value * last_value.abs()).max(self.absolute_min)
                }
                DeadbandType::Absolute => cfg.deadband_value,
            }
        } else {
            (self.default_analog_percent * last_value.abs()).max(self.absolute_min)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_first_value_always_passes() {
        let filter = DeadbandFilter::new();
        assert!(filter.should_write("dev1", "temp", 25.0).await);
    }

    #[tokio::test]
    async fn test_same_value_filtered() {
        let filter = DeadbandFilter::new();
        filter.record_write("dev1", "temp", 25.0).await;
        assert!(!filter.should_write("dev1", "temp", 25.0).await);
    }

    #[tokio::test]
    async fn test_small_change_filtered() {
        let filter = DeadbandFilter::new();
        filter.record_write("dev1", "temp", 25.0).await;
        // Default 0.1% of 25 = 0.025, absolute_min = 0.001
        assert!(!filter.should_write("dev1", "temp", 25.02).await);
    }

    #[tokio::test]
    async fn test_large_change_passes() {
        let filter = DeadbandFilter::new();
        filter.record_write("dev1", "temp", 25.0).await;
        assert!(filter.should_write("dev1", "temp", 26.0).await);
    }

    #[tokio::test]
    async fn test_absolute_deadband() {
        let filter = DeadbandFilter::new();
        filter
            .set_config(
                "dev1",
                "temp",
                DeadbandEntry {
                    deadband_type: DeadbandType::Absolute,
                    deadband_value: 1.0,
                },
            )
            .await;
        filter.record_write("dev1", "temp", 25.0).await;
        assert!(!filter.should_write("dev1", "temp", 25.5).await);
        assert!(filter.should_write("dev1", "temp", 26.1).await);
    }

    #[tokio::test]
    async fn test_batch_filtering() {
        let filter = DeadbandFilter::new();
        let records = vec![
            ("dev1".into(), 1000i64, "temp".into(), 25.0f64),
            ("dev1".into(), 2000i64, "temp".into(), 25.001f64),
            ("dev1".into(), 3000i64, "temp".into(), 26.0f64),
            ("dev2".into(), 1000i64, "pressure".into(), 100.0f64),
        ];
        let passed = filter.filter_batch(&records).await;
        assert_eq!(passed.len(), 3);
    }
}
