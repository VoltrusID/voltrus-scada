#![allow(dead_code)]
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::RwLock;
use tokio::time::{self, Duration};

use crate::models::WatchdogConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Critical,
    Stopped,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SubsystemHealth {
    pub status: HealthStatus,
    pub last_check: String,
    pub details: Option<String>,
}

/// A liveness check function. Returns Ok(details_string) or Err(error_string).
pub type HealthCheckFn = Box<dyn Fn() -> Result<String, String> + Send + Sync>;

pub struct HealthMonitor {
    checks: Arc<RwLock<HashMap<String, HealthCheckFn>>>,
    results: Arc<RwLock<HashMap<String, SubsystemHealth>>>,
    config: WatchdogConfig,
    start_time: Instant,
}

impl HealthMonitor {
    pub fn new(config: WatchdogConfig) -> Self {
        Self {
            checks: Arc::new(RwLock::new(HashMap::new())),
            results: Arc::new(RwLock::new(HashMap::new())),
            config,
            start_time: Instant::now(),
        }
    }

    /// Register a subsystem health check.
    pub async fn register(&self, name: &str, check: HealthCheckFn) {
        self.checks.write().await.insert(name.to_string(), check);
    }

    /// Run all registered health checks once.
    pub async fn run_checks(&self) {
        let checks = self.checks.read().await;
        let mut results = self.results.write().await;

        for (name, check) in checks.iter() {
            let now = crate::timeutil::now_iso();
            let entry = match check() {
                Ok(details) => SubsystemHealth {
                    status: HealthStatus::Healthy,
                    last_check: now,
                    details: Some(details),
                },
                Err(err) => {
                    tracing::warn!("Health check '{}' failed: {}", name, err);
                    SubsystemHealth {
                        status: HealthStatus::Degraded,
                        last_check: now,
                        details: Some(err),
                    }
                }
            };
            results.insert(name.clone(), entry);
        }
    }

    /// Get overall health status.
    pub async fn get_health(&self) -> (HealthStatus, HashMap<String, SubsystemHealth>) {
        let results = self.results.read().await;
        let overall =
            if results.is_empty() || results.values().all(|r| r.status == HealthStatus::Healthy) {
                HealthStatus::Healthy
            } else if results.values().any(|r| r.status == HealthStatus::Critical) {
                HealthStatus::Critical
            } else {
                HealthStatus::Degraded
            };
        (overall, results.clone())
    }

    /// Get uptime in seconds.
    pub fn uptime_secs(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }

    /// Start the periodic health check loop.
    pub fn start(self: &Arc<Self>) {
        let monitor = Arc::clone(self);
        let interval = self.config.health_check_interval_secs;
        tokio::spawn(async move {
            let mut ticker = time::interval(Duration::from_secs(interval));
            loop {
                ticker.tick().await;
                monitor.run_checks().await;
            }
        });
    }
}
