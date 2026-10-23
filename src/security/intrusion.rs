use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Tracks suspicious activity (e.g., repeated 401s) per IP for intrusion detection.
#[derive(Clone)]
pub struct IntrusionDetector {
    suspicious_401s: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    thresholds: Arc<Mutex<crate::models::SecurityThresholds>>,
}

impl IntrusionDetector {
    pub fn new() -> Self {
        Self {
            suspicious_401s: Arc::new(Mutex::new(HashMap::new())),
            thresholds: Arc::new(Mutex::new(crate::models::SecurityThresholds::default())),
        }
    }

    pub fn record_401(&self, ip: &str) {
        let thresholds = self.thresholds.lock().unwrap();
        let window_secs = thresholds.suspicious_401_window_secs;
        let threshold = thresholds.suspicious_401_threshold;
        drop(thresholds);

        let mut map = self.suspicious_401s.lock().unwrap();
        let now = Instant::now();
        let key = ip.to_string();
        let entries = map.entry(key).or_default();
        entries.retain(|t| now.duration_since(*t) < Duration::from_secs(window_secs));
        entries.push(now);

        if entries.len() >= threshold as usize {
            tracing::warn!(
                "SECURITY ALERT: {} failed auth attempts from IP {} in {}s",
                entries.len(),
                ip,
                window_secs
            );
        }
    }

    /// Load thresholds from the database and update the cached configuration.
    pub async fn load_thresholds(&self, storage: &crate::storage::Storage) {
        match storage.get_security_thresholds().await {
            Ok(t) => {
                let mut thresholds = self.thresholds.lock().unwrap();
                *thresholds = t;
            }
            Err(e) => {
                tracing::warn!("Failed to load security thresholds from DB: {}", e);
            }
        }
    }

    /// Record a 401 and persist the event to the security log database.
    pub async fn record_401_with_storage(&self, ip: &str, storage: &crate::storage::Storage) {
        self.load_thresholds(storage).await;
        self.record_401(ip);
        let _ = storage
            .insert_security_event(&crate::models::CreateSecurityEvent {
                event_type: "auth_401".into(),
                severity: "warning".into(),
                source_ip: Some(ip.to_string()),
                user_agent: None,
                username: None,
                details: Some("Unauthorized request".into()),
            })
            .await;
    }
}

impl Default for IntrusionDetector {
    fn default() -> Self {
        Self::new()
    }
}
